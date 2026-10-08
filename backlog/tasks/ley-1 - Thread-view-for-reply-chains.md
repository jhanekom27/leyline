---
id: LEY-1
title: Thread view for reply chains
status: To Do
assignee: []
created_date: '2026-10-08 02:11'
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
- [ ] #1 A command shows only the messages belonging to a given message reply chain
- [ ] #2 The view includes both ancestor messages and descendant replies in the chain
- [ ] #3 Leaving the view returns to the normal transcript
<!-- AC:END -->
