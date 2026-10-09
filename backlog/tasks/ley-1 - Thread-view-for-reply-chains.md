---
id: LEY-1
title: Thread view for reply chains
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-08 02:11'
updated_date: '2026-10-08 06:49'
labels:
  - messaging
dependencies: []
type: feature
ordinal: 1000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Following a reply chain today means scrolling the full transcript by eye. Messages already carry an optional `reply_to` id (see the Replies feature), so a dedicated view could walk that chain -- both up to what a message replied to and down to everything that replied to it -- and show just that conversation in isolation.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A command shows only the messages belonging to a given message reply chain
- [x] #2 The view includes both ancestor messages and descendant replies in the chain
- [x] #3 Leaving the view returns to the normal transcript
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add src/thread.rs: ThreadView{origin, messages} + build_chain(pool, origin) walking reply_to up (ancestors) and BFS down (descendants), sorted by (ts_unix_ms, id); cycle/dangling-parent safe. Unit tests.
2. app.rs: add Channel.thread: Option<ThreadView> and AppState.picking_thread: Option<u64>. Update visible_chat_ids/find_message to check thread first (then search, then transcript).
3. app.rs: add newest_thread_candidate, start_thread_pick, move_thread_pick, handle_thread_picking_key, open_thread, close_thread; Ctrl+T keybinding; Esc also closes an open thread view; run_command dispatches /thread -> run_thread (closes if open, else opens newest visible message's thread); run_search also clears thread (mutual exclusion); finish_activating_channel/remove_channel clear picking_thread; update run_help text.
4. ui.rs: build_thread_rows (mirrors build_search_rows, no highlighting); build_message_rows checks channel.thread before channel.search; is_picked checks include picking_thread; render_messages title gets picking_thread + channel.thread branches; add /thread to COMMAND_HINTS.
5. Update README.md (Commands table, Keybindings table, Esc row) and concept.md (Project layout) to document /thread and Ctrl+T.
6. Tests across thread.rs/app.rs/ui.rs per plan; run cargo fmt, cargo clippy, cargo test.
Chain scope decision: thread = ancestors of origin + origin + descendants of origin only (excludes sibling branches off an ancestor). Mutual exclusion: a channel shows at most one of {search, thread} at a time.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented: new src/thread.rs (ThreadView + build_chain walking reply_to up/down, cycle/dangling-safe, sorted by (ts_unix_ms, id); 10 unit tests). app.rs: Channel.thread + AppState.picking_thread fields, visible_chat_ids/find_message check thread first, Ctrl+T picking (start_thread_pick/move_thread_pick/handle_thread_picking_key), open_thread/close_thread/run_thread, /thread command, Esc also leaves an open thread view, run_search/open_thread mutually clear the other view, picking_thread cleared on channel switch/removal/join; 12 new/extended tests. ui.rs: build_thread_rows, pane title + picking_thread title branch, is_picked highlighting, COMMAND_HINTS entry; 2 new tests. Updated README.md (Commands/Keybindings tables, Esc row) and concept.md (project layout). Verified: cargo build, cargo fmt (reverted unrelated pre-existing fmt drift in src/dm_registry.rs and src/net.rs that cargo fmt surfaced but is unrelated to this task), cargo clippy --all-targets (clean), cargo test (359 passed).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added /thread (and a Ctrl+T picker mirroring Ctrl+R) to show just a message's reply chain -- ancestors (walking reply_to up) plus descendants (anything transitively replying to it), excluding unrelated sibling branches -- in place of the normal transcript. Re-running /thread or pressing Esc returns to the normal view. New src/thread.rs holds the pure chain-building logic; app.rs/ui.rs wire it in as a channel view mode mutually exclusive with /search. Verified with cargo build, cargo clippy --all-targets (clean), and cargo test (359 passed), including 21 tests exercising each acceptance criterion directly (thread::build_chain_* for ancestor/descendant/sibling-exclusion behavior, app::tests::thread_command_* and esc_closes_an_active_thread_view_without_quitting for opening/closing the view). README.md and concept.md updated to document the new command/keybinding/module.
<!-- SECTION:FINAL_SUMMARY:END -->
