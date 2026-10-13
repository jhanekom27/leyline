---
name: leyline-ui-rendering
description: Use for ratatui layout, transcript rendering, navigation, compose controls, picker visuals, modal overlays, colors, or display formatting in leyline.
---
# UI rendering
Start with `src/ui.rs`. Rendering reads `AppState` and writes ratatui widgets; it does not mutate state or perform I/O.

`src/ui/files_overlay.rs` owns the `/files` modal renderer and its formatting helpers. Match existing terminal-width handling, rendering order, and overlay clearing behavior. If behavior requires a new action or state transition, add that in `app` rather than mutating state from `ui`.

Run `cargo test`; use `cargo fmt` for layout edits.
