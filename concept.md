# leyline — Architecture

A terminal chat app: `ratatui` for the UI, `iroh` + `iroh-gossip` for the
network layer, `iroh-blobs` for history backfill, and zero self-hosted
infra -- it relies only on n0's public relay/discovery services as a NAT
punch-through fallback. See the [README](./README.md) for how to build,
run, and use it; this document covers *why* it's built this way.

## Goals

- Snappy, responsive TUI -- input never blocks on network I/O
- Chat "channels" = private gossip topics, no central server relaying messages
- A channel's name is never itself the credential -- knowing it shouldn't be enough to join (see [Room privacy](#room-privacy))
- Peers found by pasting an invite ticket (endpoint id + room secret + address hints), not a directory service
- Runs fully async on tokio

## Architecture at a glance

```
 crossterm EventStream ──┐
                         ├─> tokio::select! loop (owns AppState) ─> ratatui render
      net::NetEvent mpsc ┘        │
                                   ├─> storage.rs   (append-only chat log)
                                   └─> backfill.rs  (iroh-blobs history manifest)

 One gossip task per joined channel (net.rs), forwarding NeighborUp/Down
 and chat/announce messages into the NetEvent channel above.
        │
 iroh::Router -- dispatches incoming connections by ALPN --
   ├─ iroh-gossip  (chat messages, presence, history announcements)
   └─ iroh-blobs   (serves/fetches history manifests for backfill)
        │
 iroh::Endpoint (dial-by-EndpointId, hole-punch, relay fallback)
```

Key principle: **one owner of UI state, everything else talks to it via channels.**
Don't share `AppState` behind a `Mutex` across network tasks — message-pass into the
event loop instead. This keeps the render loop lock-free, which is most of what "snappy"
comes down to in a TUI.

## Identity & channels

- Each user has an `iroh::SecretKey`, generated once and persisted to disk
  (OS config dir, via the `directories` crate, `0600` permissions on Unix —
  `identity.rs`). This *is* their identity — no accounts, no server. Its
  public half is the endpoint's `EndpointId`, shown in the TUI header and
  embedded in every ticket.
- A channel is a local display name (e.g. `#general`) plus a 32-byte gossip
  `TopicId`. The topic is *not* a hash of the name — see
  [Room privacy](#room-privacy).
- To invite someone, share an **invite ticket** (`ticket.rs`): the
  channel's name, its `RoomSecret`, and the sharer's `EndpointAddr` (id +
  known relay/direct-address hints), base32-encoded into one string they
  paste into `/join`. This is the one piece of manual bootstrapping you
  can't avoid without a directory server — do it once per friend, not per
  session.
- `channel_registry.rs` persists every joined channel's name, room secret,
  and known peer addresses locally, so restarting rejoins and reconnects
  without re-pasting a ticket.

## Room privacy

A channel's gossip topic is `blake3("leyline-room:" + secret)`, where
`secret` is a random 32 bytes generated the moment the channel is created
(`RoomSecret::generate`, in `ticket.rs`) — never derived from the name or
from anyone's identity, both of which are effectively public (an
`EndpointId` is shown in the UI and shared in every ticket; a name is
whatever a human typed). Consequences:

- Knowing or guessing a channel's name is not enough to join it — you need
  a ticket, or to already be a recorded peer from a previous session.
- Two unrelated channels can share a display name without ever colliding
  on the same swarm, including `#general`: every fresh install generates
  its own private secret for it, isolated from every other install's
  `#general` until you `/invite` someone into it.
- The secret rides along in every ticket, so anyone already in a channel —
  not just whoever created it — can mint a new invite for it.
- If a ticket names a channel you already have locally under a different
  secret, `Net::join` rejects it with an explanatory message rather than
  silently switching that tab to a different swarm.

This is a capability-style access model, not extra encryption on top of
what iroh already provides — iroh's QUIC/TLS transport is end-to-end
encrypted and authenticated regardless of any of this. The room secret's
job is purely to gate *who can attempt to join the swarm at all*:
iroh-gossip's HyParView membership layer accepts a `Join` from anyone who
can reach an existing member, with no invite-list of its own, so an
unguessable topic is the only thing standing in for one.

## Message wire format

Kept small and versioned from day one (`message.rs`):

```rust
#[derive(Serialize, Deserialize)]
pub struct ChatMessage {
    pub v: u8,             // format version
    pub id: u64,           // random message id, for local dedupe
    pub sender: [u8; 32],  // sender's EndpointId -- gossip gives you this
                           // at delivery anyway, but embedding it lets you
                           // verify/display without an extra lookup
    pub ts_unix_ms: u64,
    pub text: String,
}
```

Everything actually broadcast over a channel's topic is wrapped in an
envelope, so the wire format can grow without disturbing `ChatMessage`
itself:

```rust
#[derive(Serialize, Deserialize)]
pub enum GossipPayload {
    Chat(ChatMessage),
    Announce(HistoryAnnounce), // see "Persistence & history backfill" below
}
```

Serialized with `postcard` (compact, no schema-drift headaches for a
personal project). Gossip is best-effort and unordered across peers, so:
- Incoming messages are deduped by `id` (a small recently-seen ring buffer per channel, in `app.rs`)
- Messages are sorted/displayed by `ts_unix_ms`, never assumed to arrive in send order

## The snappy async event loop

Don't use `ratatui`'s blocking event poll — merge everything through
`tokio::select!` (`main.rs::run`) so a keypress renders instantly
regardless of what the network is doing:

```rust
loop {
    tokio::select! {
        Some(Ok(event)) = term_events.next() => { /* app.handle_key -- pure, no I/O */ }
        Some(net_event) = net_rx.recv() => { /* app.handle_net_event, plus storage/backfill side effects */ }
        _ = tick.tick() => { /* cursor blink / relative timestamps only */ }
    }
    if dirty {
        terminal.draw(|f| ui::render(f, &app))?;
    }
}
```

Why this is fast:
- `EventStream` is async — no blocking `poll()` call stealing a thread
- Render only happens when a `dirty` flag is set, not on a fixed tick —
  the input-to-render path never waits behind an unrelated timer
- `ratatui` diffs against the previous frame buffer before writing to the
  terminal, so redraw cost is proportional to what changed, not to
  scrollback size
- Scrollback is capped per channel (`MAX_SCROLLBACK`, `app.rs`) so
  render/memory cost doesn't grow unbounded in a long-lived session

Each joined channel's gossip receive loop runs on its own `tokio::spawn`
(`net::forward_events`), forwarding parsed `NetEvent`s into one bounded
`mpsc` channel that the select loop above owns. The channel has a modest,
fixed capacity; if it's ever full, a gossip task's `send` simply awaits,
applying backpressure to that one task rather than to the render loop.

## Presence

`iroh-gossip` emits `NeighborUp`/`NeighborDown` events as peers join/leave
a channel's mesh — `net.rs` forwards these as `NetEvent::PeerJoined/Left`,
and `app.rs` uses them to drive the "online now" sidebar, scoped to
whichever channel is active. This is inherently eventually consistent
(gossip, not a membership protocol), so treat it as "who I can currently
see," not a source of truth.

## Persistence & history backfill

Gossip is live broadcast only — someone offline for a day misses
everything sent while they were away. Two pieces cover that:

- **Local log** (`storage.rs`): every message sent or received is appended
  to a per-channel, length-prefixed `postcard` log file, named by a blake3
  hash of the channel name (so an arbitrary typed name can't escape the
  storage directory). Loaded once at startup per joined channel, so
  scrollback survives a restart.
- **`iroh-blobs` backfill** (`backfill.rs`): every known message is also
  stored as its own content-addressed blob, and each channel's messages
  are listed in a canonical order (`(ts_unix_ms, id)`) as an `iroh-blobs`
  `HashSeq` manifest, so its root hash summarizes "everything I have for
  this channel" — two peers holding the same message set always compute
  the same root, regardless of arrival order. Whenever a channel gains a
  gossip neighbor (in either direction), both sides announce their current
  root (`HistoryAnnounce`, over gossip); whichever side's root differs
  fetches the manifest and any missing message blobs directly from the
  other, over the same endpoint on `iroh-blobs`' own ALPN, then merges the
  result into its transcript and local log.

Backfill only rides a channel's existing gossip topic, so it only ever
covers channels already joined — a name or ticket is still needed to join
a channel before its history can sync to you.

## Project layout

```
src/
  main.rs             // wires everything up, runs the event loop
  app.rs              // AppState, pure handle_key/handle_net_event logic
  ui.rs               // ratatui render functions, no logic
  net.rs              // iroh Endpoint + iroh-gossip actor(s), emits NetEvent
  message.rs          // ChatMessage/GossipPayload/HistoryAnnounce wire format
  identity.rs         // SecretKey load/persist
  ticket.rs           // RoomSecret + invite ticket encode/decode
  channel_registry.rs // persisted channel list, room secrets, known peers
  storage.rs          // local per-channel message log persistence
  backfill.rs         // iroh-blobs history manifests for offline backfill
```
