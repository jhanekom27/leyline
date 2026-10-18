---
id: LEY-34
title: Scroll truncated command hints on hover
status: Done
assignee:
  - '@codex'
created_date: '2026-10-17 03:45'
updated_date: '2026-10-18 14:29'
labels:
  - ux
dependencies: []
type: feature
ordinal: 77000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Long command usages in the right-hand commands panel are clipped by the fixed sidebar width, hiding arguments. Users need to read the full usage by hovering over a clipped row. Terminal hover requires mouse reporting; choose its activation behavior with the user before implementation.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Hovering a visible truncated command row automatically scrolls that row horizontally to reveal its complete usage.
- [x] #2 Short commands remain stationary; moving to another row or leaving the panel resets the previous row.
- [x] #3 Hit testing follows the rendered layout after resize and composition changes, and modal overlays suppress hover.
- [x] #4 Mouse reporting follows the agreed activation behavior and is disabled on exit and panic; wheel input remains usable.
- [x] #5 Focused state and rendering tests and README documentation cover the behavior.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Enable mouse reporting while the commands panel is shown, using /hints as the existing toggle as chosen by the user.
2. Use the shared command catalog and layout calculations for rendering and hover hit testing. Keep animation state in AppState, scrolling one cell every 174 ms with an approximately 1.2 s end pause.
3. Suppress hover while overlays, including the Ctrl+K command picker, own input. Route motion, focus loss, and wheel input in main; refresh after layout changes; restore mouse reporting on exit and panic.
4. Cover animation, reset behavior, layout/overlay handling, and terminal-mode lifetime with regression tests. Update /help, catalog description, and README; bump from upstream 0.18.0 to 0.19.0; validate formatting, tests, Clippy, and release build.

Shutdown regression fix: after the main loop finishes normally, explicitly disable `MouseReporting`, drop `TermEvents` so its crossterm reader no longer owns stdin, then drain immediately queued terminal events while raw mode is still active. The drain is bounded to events already readable (`poll(Duration::ZERO)`) and runs before ratatui restores canonical mode, preventing pre-disable SGR mouse reports from reaching zsh. Add focused unit coverage for the cleanup ordering/helper; run cargo fmt, cargo test, cargo clippy, and cargo build --release.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented hover-only horizontal scrolling with pauses and reset, shared rendering/hit-test layout, overlay suppression, focus-loss reset, and mouse reporting tied to /hints as requested. Wheel events retain Up/Down behavior. Added exit cleanup and a panic hook to disable mouse reporting. Version is 0.18.0.
Validation: cargo fmt --check and git diff --check passed; cargo test --offline passed all 1015 tests; cargo build --offline --release succeeded; cargo clippy --offline succeeded with existing dead_code warnings in channel_registry.rs and too_many_arguments in net.rs. Strict -D warnings fails on those existing warnings. UI behavior was exercised through the ratatui TestBackend, including full /sync usage visibility and multiple terminal sizes.
Ready for user testing with ./target/release/leyline. Keep In Progress pending confirmation of real-terminal hover, wheel, selection modifiers, and mode restoration (AC 4). User explicitly requested no push or PR; all changes remain local and uncommitted on feat/command-hint-hover.

User requested faster onset during local review. Removed the initial hold: scrolling now begins on the next animation tick (within 250 ms), retaining the same scroll speed and end pause. All 1015 tests pass after updating the timing regression test. Rebuilding the local release binary; no commit, push, or PR.

User requested slightly faster scrolling. Increased speed from 4 to 5 cells per second (25% faster), with scrolling starting on the next 200 ms tick. Updated the end-pause regression test; all 1015 tests pass. Changes remain local and uncommitted.

User requested another 15% scroll-speed increase. Adjusted the dedicated animation interval from 200 ms to 174 ms (approximately 15% faster), preserving an approximately 1.2 s end pause. All 1015 tests pass; no commit, push, or PR.

User approved publishing a PR after local testing and speed refinements. Preparing the final change for review; the earlier no-push instruction is superseded by the explicit PR request. AC 4 remains pending manual verification of terminal-specific wheel/selection and panic cleanup behavior.

Rebased onto current main, retaining its new keyboard command picker and shared command catalog. Hover now reads that catalog and pauses while the command picker is open. Timing remains 174 ms; version adjusted to 0.19.0 because main is already 0.18.0. Revalidating the integrated change before publishing.

Integrated validation: all 1020 tests pass. Clippy succeeds with existing warnings in channel_registry.rs, net.rs, and the upstream command picker in app.rs. The PR will note that terminal-specific wheel/selection behavior and panic cleanup still need manual smoke verification; task remains In Progress for AC 4.

Shutdown regression fix: crossterm mouse capture was being disabled correctly, but pre-disable SGR mouse reports could remain buffered when the async EventStream stopped. Once ratatui restored canonical terminal mode, zsh treated those bytes as typed input. `main` now drains ready terminal events after `run` drops `TermEvents` and mouse reporting, but before `ratatui::restore`; the first poll waits at most 10 ms for crossterm’s EventStream reader to release its lock and subsequent polls are immediate. Added two regression tests for the drain helper.

Validation: cargo fmt, cargo test (1,041 passed), cargo clippy, and cargo build --release all passed. Clippy has the same three unrelated warnings previously recorded. Manual terminal-close verification remains needed for AC #4: quit Leyline while moving the pointer over it and confirm no `...;...M` text appears at the zsh prompt.

Ctrl+C follow-up: `AppState::handle_key` still recognizes Ctrl+C and a local interactive run returned to the shell promptly, but the exit path had an unbounded `net.shutdown().await` before terminal restoration. A stalled network task could therefore leave the terminal appearing hung after Ctrl+C. Replaced it with a two-second bounded graceful shutdown; on timeout Leyline logs a warning and restores the terminal. Added async tests covering both successful and timed-out shutdown waits.

Validation: cargo fmt, cargo test (1,043 passed), cargo clippy, and cargo build --release all passed. Clippy warnings are unchanged and unrelated. Manual verification remains needed in the affected terminal: Ctrl+C should return promptly, and quitting while moving the pointer should not leak SGR mouse text.

Manual verification confirmed by the user: Ctrl+C now closes Leyline promptly in the affected terminal, and the terminal cleanup change resolved the observed behavior. This completes the terminal-specific evidence for AC #4.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented hoverable command hints with reporting scoped to the hints panel, then fixed shutdown cleanup so queued mouse reports cannot leak to zsh and network shutdown cannot indefinitely block terminal restoration. Verified by the user in the affected terminal and with cargo fmt, 1,043 passing tests, Clippy, and a release build.
<!-- SECTION:FINAL_SUMMARY:END -->
