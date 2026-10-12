---
id: LEY-25.4
title: Save the newest shared file with a bare /save
status: To Do
assignee: []
created_date: '2026-10-12 08:28'
labels:
  - files
  - ux
dependencies: []
parent_task_id: LEY-25
type: enhancement
ordinal: 37000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The `/save` command needs a hash prefix copied out of the transcript, but the common case is simply grabbing the file that was just shared. This mirrors a bare `/reply`, which targets the most recent message. Picking an older file is deliberately left to the `/files` overlay rather than adding a second picker mode.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A bare `/save` saves the most recent attachment from another peer in the active channel, reporting the file name and destination as `/save <hash-prefix>` does today
- [ ] #2 When the active channel has no attachment from another peer, a bare `/save` says so instead of doing nothing
- [ ] #3 `/save <hash-prefix>` is unchanged
<!-- AC:END -->
