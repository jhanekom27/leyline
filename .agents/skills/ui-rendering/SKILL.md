---
name: leyline-ui-rendering
description: Use for ratatui layout, transcript rendering, navigation, compose controls, picker visuals, modal overlays, colors, or display formatting in leyline.
---
# UI rendering
Start with `src/ui.rs`, the public render facade. Rendering reads `AppState` and writes ratatui widgets; it does not mutate state or perform I/O.

Use focused children when applicable:
- `src/ui/layout.rs` — frame geometry, hover hit testing, and input/staged-area sizing.
- `src/ui/navigation.rs` — header, body, channel navigation, and switcher rendering.
- `src/ui/overlays.rs` — command picker, peer details, and reusable modal layout.
- `src/ui/picker.rs` — in-terminal file picker and filter-row rendering.
- `src/ui/messages.rs` — transcript rows, markdown/search/reply formatting, and timestamps.
- `src/ui/sidebar.rs` — peers, hints, hover popups, input, and staged attachments.
- `src/ui/files_overlay.rs` — `/files` modal rendering and its formatting helpers.

Match existing terminal-width handling, rendering order, and overlay clearing behavior. If behavior requires a new action or state transition, add that in `app` rather than mutating state from `ui`. UI regression tests are in `src/ui/tests.rs`; preserve their assertions and add focused coverage there.

Run `cargo test`; use `cargo fmt` for layout edits.
