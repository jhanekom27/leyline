---
name: leyline-app-state
description: Use for leyline commands, keyboard input, channel state, transcript behavior, overlays, reply selection, thread views, and pure AppState transitions.
---
# App state
Start with `src/app.rs`, the `AppState` and `InputAction` facade. `AppState` is the single UI-state owner and returns `InputAction` values for work that belongs to runtime, networking, or the filesystem.

Use focused children when applicable:
- `src/app/state.rs` — startup-loaded state, history, channels, labels, and navigation.
- `src/app/input.rs` — keyboard dispatch and input-mode transitions for overlays and the file picker.
- `src/app/identity.rs` — locally authored messages, peer identity/presence, and save notices.
- `src/app/commands.rs` — input submission, slash-command dispatch, and general command state transitions.
- `src/app/file_actions.rs` — save, folder, size-limit, autosave, and outbox command transitions.
- `src/app/sync.rs` — sync, conflict, trash, and sync-status transitions.
- `src/app/compose.rs` — clipboard paste, attachment resolution, and message composition.
- `src/app/files_overlay.rs` — `/files` overlay state and save/open actions.
- `src/app/net_events.rs` — state changes driven by `NetEvent`.
- `src/app/replies.rs` — reply-target selection and thread views.
Do not add filesystem or network I/O here. App-state regression tests are in `src/app/tests.rs`; preserve their assertions and add focused coverage there when changing a transition. Run `cargo test`.

## Behavioral scope
Before changing a slash command based on `ChannelKind`, especially by refusing
or disabling it for a variant, verify the behavior's scope first. Read the
related Backlog task tree, inspect the command's existing `ChannelKind` tests,
and follow the runtime or network path that makes it work. A documentation
task's wording alone is not evidence that another channel kind is unsupported.
