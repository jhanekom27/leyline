---
id: LEY-6
title: Unread counts and jump-to-first-unread
status: To Do
assignee: []
created_date: '2026-10-08 02:11'
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
- [ ] #1 The tab bar shows how many unread messages a channel has, not just whether it has any
- [ ] #2 Switching to a channel with unread messages scrolls to the first unseen message
- [ ] #3 Unread state clears appropriately once those messages have been viewed
<!-- AC:END -->
