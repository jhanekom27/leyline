---
name: leyline-app-state
description: Use for leyline commands, keyboard input, channel state, transcript behavior, overlays, reply selection, thread views, and pure AppState transitions.
---
# App state
Start with `src/app.rs`. `AppState` is the single UI-state owner and returns `InputAction` values for work that belongs to runtime, networking, or the filesystem.

Use focused children when applicable:
- `src/app/files_overlay.rs` — `/files` overlay state and save/open actions.
- `src/app/net_events.rs` — state changes driven by `NetEvent`.
- `src/app/replies.rs` — reply-target selection and thread views.

Do not add filesystem or network I/O here. Preserve inline tests in the responsibility’s current module and run `cargo test`.
