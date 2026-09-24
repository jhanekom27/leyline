# leyline

A peer-to-peer terminal chat app. No servers, no accounts, no central relay
to run -- your identity is a local keypair, "channels" are private gossip
topics (each with its own randomly-generated key, not a guessable hash of
its name), and messages travel directly between peers using
[iroh](https://github.com/n0-computer/iroh) + iroh-gossip, all inside a
[ratatui](https://ratatui.rs) TUI.

## Status: private channels, with history backfill for offline peers

`leyline` channels are private by default: creating or first joining one
(by name or ticket) generates a random secret that -- not the name you
typed -- actually determines its gossip topic, so a channel is only
reachable by someone you've handed a ticket to, and two people who happen
to pick the same name never end up in the same swarm. `leyline` also
backfills channel history for peers that were offline (or join a channel
fresh): whenever a channel gains a gossip neighbor, and again periodically
regardless of neighbor churn, every peer floods its current history's
root hash -- an `iroh-blobs` content manifest -- to the whole channel,
and whoever's behind automatically fetches the messages they're missing
directly from whoever announced them, no manual command needed -- not
just from whichever peer you happened to connect through. Combined with
local persistence, multi-channel support (one gossip task per joined
topic, a tab bar to switch between them, and a presence sidebar scoped to
whichever channel is active), a couple of things are still deliberately
minimal:

- **Identity persists across runs** -- your keypair is generated once and
  saved to disk, so your endpoint id stays the same every time you start
  `leyline` (see [Build & run](#build--run) for how this affects local
  multi-instance testing).
- **A channel's name is just a local label, not a credential** -- what
  actually gates entry is its random secret, carried inside its invite
  ticket. Guessing or reusing a name -- even `#general`, which every fresh
  install starts with its own private copy of -- never joins you to
  someone else's channel.
- **Channel membership and known peers persist too** -- every channel you
  join (by name or ticket) and the addresses of peers you've talked to
  are remembered on disk, so restarting `leyline` rejoins all of them and
  reconnects automatically, without re-pasting an invite ticket. An
  invite ticket itself is one-directional -- it bundles *your* current
  address so someone else can bootstrap onto you -- so pasting your own
  ticket back in after a restart can't reconnect you to anyone; it's
  detected and ignored in favor of previously-known peers for that
  channel.
- **Backfill only covers channels you've joined** -- it rides each
  channel's existing gossip topic, so you still need a name or ticket to
  join a channel before its history can sync to you.

See [Roadmap](#roadmap) below for how it got here.

## Requirements

- A recent stable Rust toolchain with edition 2024 support (Rust 1.91 or newer -- iroh-gossip's current MSRV)

## Build & run

```sh
cargo build --release
cargo run
```

On startup, `leyline` prints an invite ticket for each channel it joined
(before the TUI takes over -- just "#general" by default, and private to
this instance alone until you share it) and shows your endpoint id in the
TUI header (`you: <hex id>`). To have a second instance join the same
channel and talk to the first, run it the same way in another terminal:

```sh
cargo run
```

then paste that printed ticket into its message box:

```
/join <ticket printed by the first instance>
```

The ticket bundles the channel's name, its private room secret, and the
sharer's address (including relay/direct-address hints), so pasting it is
enough to connect -- no separate directory service needed, and no one who
only knows (or guesses) the channel's name can join, since the name never
determines the swarm. Connecting still relies on iroh's default relay
servers for NAT traversal, so all instances need outbound internet access.

Once running, join or create additional channels from the message box with
`/join <channel-name-or-ticket>`: a bare name (e.g. `/join project-x`)
creates a brand-new, private channel under that name locally -- share its
ticket with others via `/invite` -- while pasting someone else's ticket
joins their swarm directly. Switch between joined channels with `Tab` /
`Shift+Tab`.

Your identity now persists in your OS config directory, keyed by your `$HOME`.
Chat history persists the same way in your OS data directory -- one log
file per joined channel, so scrollback survives a restart.
Running two instances under the *same* `$HOME` on one machine loads the
*same* identity for both -- and iroh rejects a peer connecting to itself --
so for local multi-instance testing, give each instance its own fake home,
e.g.:

```sh
HOME=/tmp/leyline-a cargo run
HOME=/tmp/leyline-b cargo run
```

...then `/join` the first instance's printed `#general` ticket from the
second, as above.

Each fake `$HOME` also redirects Cargo's own cache, so the first build under
a new one re-fetches and rebuilds every dependency. To avoid that, capture
your real cargo home first and pass it through explicitly:

```sh
REAL_CARGO_HOME="$HOME/.cargo"
HOME=/tmp/leyline-a CARGO_HOME="$REAL_CARGO_HOME" cargo run
HOME=/tmp/leyline-b CARGO_HOME="$REAL_CARGO_HOME" cargo run
```

## Keybindings

| Key | Action |
| --- | --- |
| Type | Insert a character at the cursor |
| `Left` / `Right` | Move the cursor one character |
| `Home` / `Ctrl+A` | Jump to the start of the line |
| `End` / `Ctrl+E` | Jump to the end of the line |
| `Backspace` / `Delete` | Delete the character before / at the cursor |
| `Ctrl+U` | Delete from the start of the line to the cursor |
| `Ctrl+K` | Delete from the cursor to the end of the line |
| `Ctrl+W` | Delete the word before the cursor |
| `Enter` | Send the composed message, or run a `/` command |
| (paste, e.g. `Cmd+V`/`Ctrl+V`) | Paste text from the terminal as one atomic edit, so a multi-line paste can't prematurely send a partial line or trigger a stray `/command` |
| `Ctrl+V` | Share whatever's on the OS clipboard (a file, an image, or text) with the active channel -- same as `/paste`. `Cmd+V` also works if your terminal forwards it, but most (including this one) reserve it for their own text-paste above |
| `Ctrl+R` | Pick a message to reply to -- `Up`/`Down` move the highlight, `Enter` confirms, `Esc` cancels |
| `Up` / `Down` | Scroll the message history (or move the highlight while picking a reply, see `Ctrl+R`) |
| `Tab` / `Shift+Tab` | Switch to the next / previous joined channel |
| `Esc` / `Ctrl+C` | Cancel an in-progress reply pick or armed reply first, if any; otherwise quit |

## Commands

Typed into the message box and submitted with `Enter`:

| Command | Action |
| --- | --- |
| `/join <channel-name>` | Join or create a channel by name, then switch to it |
| `/join <ticket>` | Join the channel named in a pasted invite ticket |
| `/invite` | Show the active channel's invite ticket in the transcript (and copy it to your clipboard, if one is available) to share with others |
| `/leave` | Leave the active channel |
| `/leave <channel-name>` | Leave a specific joined channel without switching to it |
| `/who` | List the active channel's peers, showing each one's full endpoint id, broadcast nickname (if any), and local alias (if any) |
| `/send <path>` | Share a local file with the active channel; peers see its name and size and can fetch it with `/save`, never automatically |
| `/save <hash-prefix>` | Download a file previously shared in any joined channel to your Downloads folder (in a `leyline` subfolder) |
| `/paste` | Share whatever's on the OS clipboard with the active channel: a real file (e.g. copied in Finder/Explorer, including an animated GIF, byte-for-byte) if there is one, else a screenshot or other copied image (always shared as a single static PNG frame -- no OS clipboard format carries multi-frame/animation data), else plain copied text as a chat message. Same as the `Ctrl+V` keybinding |
| `/alias <hex-prefix> <name>` | Assign a local pet name to the peer whose endpoint id starts with `<hex-prefix>`, shown in place of their hex id from then on (local only, never sent to peers) |
| `/nick <name>` | Broadcast a chosen display name to every joined channel (spoofable -- shown as `name (hex-prefix)` until you `/alias` that peer) |
| `/search <term>` (alias `/s`) | Filter the active channel's messages down to ones containing `<term>` (instant for what's loaded, extended in the background with a scan of the full on-disk history); the pane title reminds you it's active and that `/search`/`/s` with no argument clears it |
| `/reply` | Arm a reply to the most recent message in the active channel -- type normally afterward to send it as a reply (or press `Ctrl+R` instead to pick an older or specific-sender message) |
| `/reply <text>` | Arm and immediately send `<text>` as a reply to the most recent message, in one step |
| `/hints` | Toggle the sidebar's command hints panel on or off |
| `/help` | Show the full list of commands and keybindings |

## Roadmap

Rough build order this project followed, from a `ratatui`-only prototype to
the full peer-to-peer app -- all shipped:

- [x] 1. `ratatui` chat UI against fake/local messages -- validate layout and keybindings
- [x] 2. wire up `iroh` + `iroh-gossip` so two local instances can talk over one gossip topic
- [x] 3. Invite ticket generation/parsing, identity persistence
- [x] 4. Multi-channel support (one gossip task per joined topic), presence sidebar
- [x] 5. Local message persistence + reload on start
- [x] 6. `iroh-blobs`-based history backfill for offline peers
- [x] 7. Room privacy: per-channel random secrets instead of name-derived topics

## Architecture

See [`concept.md`](./concept.md) for the architecture notes: wire format,
the room-privacy model, event loop design, module layout, and the
reasoning behind the network/UI split.
