# leyline

A peer-to-peer terminal chat app. No servers, no accounts, no central relay
to run -- your identity is a local keypair, "channels" are gossip topics,
and messages travel directly between peers using
[iroh](https://github.com/n0-computer/iroh) + iroh-gossip, all inside a
[ratatui](https://ratatui.rs) TUI.

## Status: early skeleton

Right now `leyline` is a **local-only TUI shell**: it renders the chat
layout against a few hard-coded fake messages so the UI and keybindings can
be validated before any networking is wired up. There is no peer-to-peer
messaging yet -- see [Roadmap](#roadmap) below for what's next.

## Requirements

- A recent stable Rust toolchain with edition 2024 support (Rust 1.85 or newer)

## Build & run

```sh
cargo build --release
cargo run
```

## Keybindings

| Key | Action |
| --- | --- |
| Type | Compose a message in the input box |
| `Enter` | Send the composed message |
| `Backspace` | Edit the composed message |
| `Up` / `Down` | Scroll the message history |
| `Esc` / `Ctrl+C` | Quit |

## Roadmap

Rough build order (see `concept.md` for the full design doc):

- [x] 1. `ratatui` chat UI against fake/local messages -- validate layout and keybindings
- [ ] 2. **Up next:** wire up `iroh` + `iroh-gossip` so two local instances can talk over one gossip topic
- [ ] 3. Invite ticket generation/parsing, identity persistence
- [ ] 4. Multi-channel support (one gossip task per joined topic), presence sidebar
- [ ] 5. Local message persistence + reload on start
- [ ] 6. (Stretch) `iroh-blobs`-based history backfill for offline peers

## Architecture

See [`concept.md`](./concept.md) for the full design doc: wire format, event
loop design, module layout, and the reasoning behind the network/UI split.
