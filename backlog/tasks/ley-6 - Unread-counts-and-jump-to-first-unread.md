---
id: LEY-6
title: Unread counts and jump-to-first-unread
status: Done
assignee:
  - '@me'
created_date: '2026-10-08 02:11'
updated_date: '2026-10-17 13:49'
labels:
  - channels
dependencies: []
type: feature
ordinal: 6000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
`Channel::has_unread` is a bool today, shown as a single dot in the tab bar, so a channel with one new message looks the same as one with a hundred. Switching to a channel also lands wherever `scroll` last was rather than at the oldest unseen message.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The tab bar shows how many unread messages a channel has, not just whether it has any
- [x] #2 Switching to a channel with unread messages scrolls to the first unseen message
- [x] #3 Unread state clears appropriately once those messages have been viewed
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented transient unread counts with a first-message viewport anchor. Verified with cargo fmt, cargo test (1023 passed), cargo clippy (3 pre-existing warnings), and cargo build --release.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced boolean unread markers with numeric counts, jump to the first unread message when activating a channel, and clear unread state after activation. Verified by focused app/UI tests plus cargo test (1023 passed), cargo clippy, and cargo build --release.
<!-- SECTION:FINAL_SUMMARY:END -->
