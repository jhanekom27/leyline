# P2P TUI Chat â€” Design Doc

A terminal chat app: `ratatui` for the UI, `iroh` + `iroh-gossip` for the network layer,
zero self-hosted infra (relies on n0's public relay/discovery services only for NAT
punch-through fallback).

## Goals

- Snappy, responsive TUI â€” input never blocks on network I/O
- Chat "channels" = gossip topics, no central server relaying messages
- Peers found by pasting a one-time invite ticket (NodeId + topic), not by a directory service
- Runs fully async on tokio

## Crates

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
iroh = "0.36"
iroh-gossip = "0.36"
ratatui = "0.29"
crossterm = { version = "0.28", features = ["event-stream"] }
serde = { version = "1", features = ["derive"] }
postcard = { version = "1", features = ["alloc"] }
blake3 = "1"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = "0.3"
directories = "5"
futures = "0.3"
```

(Pin exact iroh/iroh-gossip versions to whatever's current when you start â€” the API has
been moving fast through 2026; check crates.io before locking in.)

## Architecture at a glance

```
 crossterm EventStream â”€â”
                         â”œâ”€> tokio::select! loop (owns AppState) â”€> ratatui render
 network mpsc channel â”€â”€â”˜
        â–²
        â”‚ NeighborUp/Down, GossipMessage
        â”‚
 iroh-gossip topic task (one per joined channel)
        â”‚
 iroh::Endpoint (dial-by-NodeId, hole-punch, relay fallback)
```

Key principle: **one owner of UI state, everything else talks to it via channels.**
Don't share `AppState` behind a `Mutex` across network tasks â€” message-pass into the
event loop instead. This keeps the render loop lock-free, which is most of what "snappy"
comes down to in a TUI.

## Identity & channels

- Each user has an `iroh::SecretKey`, generated once and persisted to disk (e.g.
  `~/.config/p2p-chat/identity` via the `directories` crate). This *is* their identity â€”
  no accounts, no server.
- A "channel" is just a 32-byte gossip `TopicId`. Derive it deterministically from a
  human name so friends can agree on it out of band:
  `TopicId::from(blake3::hash(b"channel:my-friend-group"))`.
- To invite someone, share an **invite ticket**: your `NodeId` (+ known relay/addr hints)
  and the `TopicId`, base32-encoded into one string they paste in. This is the one piece
  of manual bootstrapping you can't avoid without a directory server â€” do it once per
  friend, not per session.
- Store learned peer addresses per channel locally so reconnecting later doesn't require
  re-pasting tickets (iroh's endpoint will remember/re-resolve via discovery too).

## Message wire format

Keep it small and versioned from day one:

```rust
#[derive(Serialize, Deserialize)]
struct ChatMessage {
    v: u8,              // format version
    id: u64,             // random message id, for local dedupe
    sender: [u8; 32],     // sender's NodeId â€” gossip gives you this at delivery anyway,
                          // but embedding it lets you verify/display without extra lookups
    ts_unix_ms: u64,
    text: String,
}
```

Serialize with `postcard` (compact, no schema drift headaches for a personal project).
Gossip is best-effort and unordered across peers, so:
- Dedupe incoming messages by `id` (keep a small LRU set of recently seen ids)
- Sort/display by `ts_unix_ms`, don't assume delivery order == send order

## The snappy async event loop

This is the part worth getting right. Don't use `ratatui`'s blocking event poll â€” merge
everything through `tokio::select!` so a keypress renders instantly regardless of what
the network is doing:

```rust
use crossterm::event::{Event, EventStream};
use futures::StreamExt;
use tokio::time::{interval, Duration};

let mut term_events = EventStream::new();
let mut net_events: mpsc::Receiver<NetEvent> = /* from gossip task */;
let mut tick = interval(Duration::from_millis(250)); // cursor blink / relative timestamps only
let mut dirty = true;

loop {
    tokio::select! {
        Some(Ok(ev)) = term_events.next() => {
            if let Event::Key(key) = ev {
                app.handle_key(key); // pure, fast, no I/O
                dirty = true;
            }
        }
        Some(net_ev) = net_events.recv() => {
            app.handle_net_event(net_ev); // push message into scrollback, update presence
            dirty = true;
        }
        _ = tick.tick() => {
            // only redraw on tick if something like a cursor/timestamp needs it
        }
    }

    if dirty {
        terminal.draw(|f| ui::render(f, &app))?;
        dirty = false;
    }
}
```

Notes on why this is fast:
- `EventStream` is async â€” no blocking `poll()` call stealing a thread
- Render only happens when state actually changed (dirty flag), not on a fixed tick â€”
  avoids wasted redraws, and more importantly avoids the inputâ†’render path ever waiting
  behind an unrelated timer
- `ratatui` already diffs against the previous frame buffer before writing to the
  terminal, so redraw cost is proportional to what changed on screen, not scrollback size
- Cap in-memory scrollback per channel (e.g. last 500 messages in a `VecDeque`, older
  messages paged from local storage on scroll-up) so render cost doesn't grow unbounded
  in a long-lived session

Run the iroh endpoint + one gossip-receive task per joined topic on their own
`tokio::spawn`s, each forwarding parsed `NetEvent`s into the single `net_events` channel.
Use a bounded channel with a small buffer; if it's ever full, drop-oldest rather than
letting a slow UI loop apply backpressure into the network task.

## Presence

`iroh-gossip` emits `NeighborUp`/`NeighborDown` events as peers join/leave the topic's
mesh â€” forward these as `NetEvent::PeerJoined/Left` and use them to drive an "online
now" list in the sidebar. This is inherently eventually-consistent (gossip, not a
membership protocol), so treat it as "who I can currently see," not a source of truth.

## Persistence & offline history (v2)

Gossip is live broadcast only â€” someone offline for a day misses everything sent while
they were away. For v1, accept that and just persist your own local log of what you
*did* receive (flat file or `sqlite` via `rusqlite`, one row per `ChatMessage`, keyed by
`id`) so scrollback survives restarts.

For actual backfill later, `iroh-blobs` (content-addressed blob sync, same ecosystem)
is the natural next step: on reconnect, ask a peer for their log's latest hash and pull
the delta. Worth treating as a separate milestone rather than blocking v1 on it.

## Suggested build order

1. `ratatui` chat UI against fake/local messages â€” validate the layout and keybindings
2. Wire up `iroh` + `iroh-gossip`, two local instances talking over one topic
3. Invite ticket generation/parsing, identity persistence
4. Multi-channel support (one gossip task per joined topic), presence sidebar
5. Local message persistence + reload on start
6. (Stretch) `iroh-blobs`-based history backfill for offline peers

## Project layout

```
src/
  main.rs        // wires everything up, runs the event loop
  app.rs          // AppState, pure handle_key/handle_net_event logic
  ui.rs           // ratatui render functions, no logic
  net.rs          // iroh Endpoint + iroh-gossip actor(s), emits NetEvent
  message.rs      // ChatMessage, dedupe/ordering helpers
  identity.rs      // SecretKey load/persist
  ticket.rs        // invite ticket encode/decode
  storage.rs       // local message log persistence
```