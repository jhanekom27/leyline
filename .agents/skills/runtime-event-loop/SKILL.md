---
name: leyline-runtime-event-loop
description: Use for application startup, shutdown, the tokio select loop, event/action routing, asynchronous save jobs, terminal events, or pairing restart behavior in leyline.
---
# Runtime event loop
Start with `src/main.rs`. It initializes services and owns the `tokio::select!` loop that routes terminal events, network events, background results, and timers.

`src/runtime.rs` contains side-effect helpers: send routing, save jobs, attachment staging, pasted-file handling, history announcements, and bell signaling. Keep this layer responsible for I/O side effects and preserve event-loop responsiveness.

Run `cargo test`, then `cargo build --release` when changing startup or runtime routing.
