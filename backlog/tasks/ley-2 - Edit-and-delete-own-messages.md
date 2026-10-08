---
id: LEY-2
title: Edit and delete own messages
status: To Do
assignee: []
created_date: '2026-10-08 02:11'
labels:
  - messaging
dependencies: []
type: feature
ordinal: 2000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
There is currently no way to correct or retract a sent message. Any edit/delete mechanism needs to fit the existing append-only gossip history and backfill model (`backfill.rs`), so history never ends up with holes that dedupe or backfill logic has to special-case.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A user can delete a message they sent; it is replaced with a visible "(deleted)" placeholder instead of disappearing
- [ ] #2 A user can edit a message they sent, and peers see the updated content
- [ ] #3 A user cannot edit or delete a message sent by someone else
- [ ] #4 Backfill and dedupe continue to work correctly for channels containing edited or deleted messages
<!-- AC:END -->
