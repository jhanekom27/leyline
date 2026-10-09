---
id: LEY-18
title: Auto-reply within an open thread view
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-09 09:25'
updated_date: '2026-10-09 09:33'
labels:
  - messaging
dependencies: []
type: feature
ordinal: 18000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up to LEY-1 (Thread view for reply chains). Today, viewing a thread (/thread) only filters what's displayed -- a plain typed message sent while the thread view is open still has no reply_to unless the user explicitly arms one with Ctrl+R or /reply, so it lands at the top level of the transcript instead of staying part of the conversation being viewed. A message sent while reading a thread should automatically stay in that thread.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Sending a plain (non-explicitly-armed) message while a thread view is open attaches it to that thread, so it appears in the chain next time the thread is viewed
- [x] #2 Explicitly arming a reply target with Ctrl+R or /reply while a thread view is open still takes precedence over the automatic thread target
- [x] #3 Sending a plain message when no thread view is open is unaffected (no implicit reply_to)
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented via AppState::active_thread_reply_target (submit_input falls back to it only when no explicit replying_to is armed, so Ctrl+R/`/reply` always wins) + a ui.rs input-box hint when it applies. Updated README's /thread row. Tests: sending_a_plain_message_while_viewing_a_thread_replies_within_it (AC1), explicit_reply_target_wins_over_the_automatic_thread_target (AC2), sending_a_plain_message_without_a_thread_view_has_no_reply_to (AC3) -- all pass. Full suite: 362 passed. cargo clippy --all-targets clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Sending a plain message while a /thread view is open now automatically replies to the newest message in that thread (AppState::active_thread_reply_target, consulted by submit_input only when no reply is explicitly armed), so it stays part of the conversation instead of landing at the top level. An explicit Ctrl+R/`/reply` target still takes precedence. The input box shows 'replying within this thread' when the implicit target applies. Verified with cargo clippy --all-targets (clean) and cargo test (362 passed), including three tests exercising each acceptance criterion directly. README's /thread row updated to document the behavior.
<!-- SECTION:FINAL_SUMMARY:END -->
