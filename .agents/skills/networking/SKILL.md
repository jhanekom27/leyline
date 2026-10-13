---
name: leyline-networking
description: Use for iroh endpoints, gossip channels, direct messaging, pairing, tickets, wire events, history backfill, or peer lifecycle changes in leyline.
---
# Networking
Start with `src/net.rs` and `concept.md`’s networking sections. `Net` owns endpoint, gossip, direct, pairing, and background task lifetimes; it reports results through `NetEvent` instead of mutating `AppState`.

`src/net/protocol.rs` owns ticket parsing, private-topic derivation, self-ticket detection, and clipboard image encoding. Preserve wire types in `src/message.rs`, ticket semantics in `src/ticket.rs`, and task cleanup on channel leave.

Run `cargo test` and `cargo clippy` for networking changes.
