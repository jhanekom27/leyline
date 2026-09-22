# leyline

A peer-to-peer terminal chat app. No servers, no accounts, no central relay
to run -- your identity is a local keypair, "channels" are gossip topics,
and messages travel directly between peers using
[iroh](https://github.com/n0-computer/iroh) + iroh-gossip, all inside a
[ratatui](https://ratatui.rs) TUI.

## Status: real gossip networking, temporary bootstrapping

`leyline` now wires up real `iroh` + `iroh-gossip` networking: two instances
can join the same default "#general" channel and exchange messages. A few
things are still deliberately minimal until later roadmap steps land:

- **Identity is ephemeral** -- a new keypair is generated every run, so your
  endpoint id changes each time (persistence is step 3).
- **Bootstrapping peers is manual** -- there's no invite ticket system yet,
  so a second instance connects with a `--connect <endpoint-id>` flag
  instead of pasting a ticket (see [Build & run](#build--run)).
- **One channel** -- everyone joins the same hard-coded "#general" topic;
  multi-channel support is a later step.

See [Roadmap](#roadmap) below for what's next.

## Requirements

- A recent stable Rust toolchain with edition 2024 support (Rust 1.91 or newer -- iroh-gossip's current MSRV)

## Build & run

```sh
cargo build --release
cargo run
```

Your endpoint id is shown in the TUI header (`you: <hex id>`). To have a
second instance join the same channel and talk to the first, copy that id
into a `--connect` flag in another terminal:

```sh
cargo run -- --connect <hex id from the first instance>
```

`--connect` can be repeated to dial multiple peers on startup. Connecting
relies on iroh's default discovery/relay services, so all instances need
outbound internet access.

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
| `Enter` | Send the composed message |
| `Up` / `Down` | Scroll the message history |
| `Esc` / `Ctrl+C` | Quit |

## Roadmap

Rough build order (see `concept.md` for the full design doc):

- [x] 1. `ratatui` chat UI against fake/local messages -- validate layout and keybindings
- [x] 2. wire up `iroh` + `iroh-gossip` so two local instances can talk over one gossip topic
- [ ] 3. **Up next:** Invite ticket generation/parsing, identity persistence
- [ ] 4. Multi-channel support (one gossip task per joined topic), presence sidebar
- [ ] 5. Local message persistence + reload on start
- [ ] 6. (Stretch) `iroh-blobs`-based history backfill for offline peers

## Architecture

See [`concept.md`](./concept.md) for the full design doc: wire format, event
loop design, module layout, and the reasoning behind the network/UI split.
