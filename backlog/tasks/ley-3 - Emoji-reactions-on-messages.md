---
id: LEY-3
title: Emoji reactions on messages
status: To Do
assignee: []
created_date: '2026-10-08 02:11'
labels:
  - messaging
dependencies: []
type: feature
ordinal: 3000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Peers have no lightweight way to acknowledge a message without sending a full reply. A reaction tied to a message id, broadcast the same way replies are, covers this with minimal UI and protocol surface.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A user can react to any visible message with an emoji
- [ ] #2 Reactions sent by peers appear inline under the message they target
- [ ] #3 Multiple reactions on the same message, including from multiple peers, are all visible
<!-- AC:END -->
