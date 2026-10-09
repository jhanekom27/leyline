---
id: LEY-19
title: Live-update an open thread view as messages join it
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-09 13:44'
updated_date: '2026-10-09 13:47'
labels:
  - messaging
dependencies: []
type: feature
ordinal: 19000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow-up to LEY-18 (Auto-reply within an open thread view). LEY-18 made a plain message sent while viewing a thread automatically reply within it, but ThreadView is still a point-in-time snapshot (like /search), so the sender has to close and reopen /thread to actually see their own new message appear -- it doesn't show up while they stay in the view. The view should grow live as directly-connected messages are sent or received, not just update on the next explicit rebuild.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Sending a plain message while a thread view is open shows that message in the view immediately, without re-running /thread or Ctrl+T
- [x] #2 An incoming message from a peer that replies to a message already in the open thread view appears in it immediately
- [x] #3 A message unrelated to the open thread (no reply_to, or replying to something outside the view) does not appear in it
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented via ThreadView::try_append (thread.rs): appends a message if its reply_to matches something already in the view, re-sorting by (ts_unix_ms, id); dedupes defensively. Wired into app::Channel::push, which every chat-message-adding path already goes through (send_text, record_sent_message, NetEvent::Received, seed_history), so both locally-sent and incoming replies update the open view immediately with no extra call sites needed. Tests: 6 unit tests on try_append (thread.rs) plus sending_a_plain_message_while_viewing_a_thread_shows_up_immediately (AC1), a_received_reply_to_a_message_in_the_open_thread_appears_immediately (AC2), an_unrelated_received_message_does_not_join_the_open_thread (AC3) -- all pass. Full suite: 371 passed. cargo clippy --all-targets clean. Updated ThreadView's and Channel.thread's doc comments to describe the new live behavior and its one known gap (an incoming reply arriving before its own parent).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
An open /thread view now grows live: ThreadView::try_append (thread.rs) appends any new chat message whose reply_to matches something already in the view, re-sorted by (ts_unix_ms, id). It's wired into Channel::push, the single choke point every chat message already passes through (local sends, file shares, incoming peer messages, startup history), so a reply made or received while reading a thread shows up immediately -- no more closing and reopening /thread to see it. The one remaining gap (an incoming reply arriving before its own parent) is documented on ThreadView and left for the next explicit rebuild, same as before this existed. Verified with cargo clippy --all-targets (clean) and cargo test (371 passed), including 9 new tests covering try_append directly and all three acceptance criteria end-to-end.
<!-- SECTION:FINAL_SUMMARY:END -->
