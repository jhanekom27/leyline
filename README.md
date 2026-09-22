# leyline

A peer-to-peer terminal chat app. No servers, no accounts, no central relay
to run -- your identity is a local keypair, "channels" are gossip topics,
and messages travel directly between peers using
[iroh](https://github.com/n0-computer/iroh) + iroh-gossip, all inside a
[ratatui](https://ratatui.rs) TUI.

## Status: history backfill for offline peers

`leyline` now backfills channel history for peers that were offline (or
join a channel fresh): whenever a channel gains a gossip neighbor, both
sides announce their current history's root hash -- an `iroh-blobs`
content manifest -- and whichever side is behind automatically fetches
the messages it's missing directly from the other, no manual command
needed. Combined with local persistence, multi-channel support (one
gossip task per joined topic, a tab bar to switch between them, and a
presence sidebar scoped to whichever channel is active), a couple of
things are still deliberately minimal:

- **Identity persists across runs** -- your keypair is generated once and
  saved to disk, so your endpoint id stays the same every time you start
  `leyline` (see [Build & run](#build--run) for how this affects local
  multi-instance testing).
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

See [Roadmap](#roadmap) below for what's next.

## Requirements

- A recent stable Rust toolchain with edition 2024 support (Rust 1.91 or newer -- iroh-gossip's current MSRV)

## Build & run

```sh
cargo build --release
cargo run
```

On startup, `leyline` prints an invite ticket for each channel it joined
(before the TUI takes over -- just "#general" by default) and shows your
endpoint id in the TUI header (`you: <hex id>`). To have a second instance
join the same channel and talk to the first, copy that printed ticket into
a `--join` flag in another terminal:

```sh
cargo run -- --join <ticket printed by the first instance>
```

The ticket bundles the channel's name and the sharer's address (including
relay/direct-address hints), so `--join` alone is enough to connect --
no separate directory service needed. Connecting still relies on iroh's
default relay servers for NAT traversal, so all instances need outbound
internet access.

Once running, join or create additional channels from the message box with
`/join <channel-name-or-ticket>`: a bare name (e.g. `/join project-x`)
joins/creates that channel locally -- share its ticket with others via
`/invite` -- while pasting someone else's ticket joins their swarm
directly. Switch between joined channels with `Tab` / `Shift+Tab`.

Your identity now persists in your OS config directory, keyed by your `$HOME`.
Chat history persists the same way in your OS data directory -- one log
file per joined channel, so scrollback survives a restart.
Running two instances under the *same* `$HOME` on one machine loads the
*same* identity for both -- and iroh rejects a peer connecting to itself --
so for local multi-instance testing, give each instance its own fake home,
e.g.:

```sh
HOME=/tmp/leyline-a cargo run
HOME=/tmp/leyline-b cargo run -- --join <ticket from the first instance>
```

Each fake `$HOME` also redirects Cargo's own cache, so the first build under
a new one re-fetches and rebuilds every dependency. To avoid that, capture
your real cargo home first and pass it through explicitly:

```sh
REAL_CARGO_HOME="$HOME/.cargo"
HOME=/tmp/leyline-a CARGO_HOME="$REAL_CARGO_HOME" cargo run
HOME=/tmp/leyline-b CARGO_HOME="$REAL_CARGO_HOME" cargo run -- --join <ticket from the first instance>
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
| `Up` / `Down` | Scroll the message history |
| `Tab` / `Shift+Tab` | Switch to the next / previous joined channel |
| `Esc` / `Ctrl+C` | Quit |

## Commands

Typed into the message box and submitted with `Enter`:

| Command | Action |
| --- | --- |
| `/join <channel-name>` | Join or create a channel by name, then switch to it |
| `/join <ticket>` | Join the channel named in a pasted invite ticket |
| `/invite` | Show the active channel's invite ticket in the transcript (and copy it to your clipboard, if one is available) to share with others |

## Roadmap

Rough build order (see `concept.md` for the full design doc):

- [x] 1. `ratatui` chat UI against fake/local messages -- validate layout and keybindings
- [x] 2. wire up `iroh` + `iroh-gossip` so two local instances can talk over one gossip topic
- [x] 3. Invite ticket generation/parsing, identity persistence
- [x] 4. Multi-channel support (one gossip task per joined topic), presence sidebar
- [x] 5. Local message persistence + reload on start
- [x] 6. (Stretch) `iroh-blobs`-based history backfill for offline peers

## Architecture

See [`concept.md`](./concept.md) for the full design doc: wire format, event
loop design, module layout, and the reasoning behind the network/UI split.
