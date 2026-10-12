---
id: LEY-26.2
title: 'One-way outbox folder: drop a file in to share it'
status: To Do
assignee: []
created_date: '2026-10-12 08:29'
labels:
  - files
  - ux
dependencies:
  - LEY-25.6
parent_task_id: LEY-26
type: feature
ordinal: 42000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Sharing a file from the file manager currently means switching to leyline and picking or pasting it. A watched `send/` folder per channel turns sharing into a drag in Finder or a `cp` on Linux. It is deliberately one-way and not synced, so there are no conflicts or echo loops: each file becomes an ordinary attachment message, and history, backfill, and replies work unchanged. Auto-save covers the receiving side.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/outbox on` and `/outbox off` switch the outbox for the active channel; it is off by default, persists across restarts, and enabling it creates `<channel folder>/send/` and reports its location
- [ ] #2 A new file appearing in `send/` is shared with the channel as an attachment once it has stopped changing, so a file still being copied is never sent half-written
- [ ] #3 Files already in `send/` when the outbox is first enabled are treated as already shared and are not sent
- [ ] #4 Hidden files and common temporary or partial files (such as `.DS_Store`, `*.part`, `*.crdownload`, `~$*`) are ignored
- [ ] #5 A file is never shared twice, including after a restart; editing it after it was shared does not send it again
- [ ] #6 Files over the `/send` size limit, or unreadable ones, are reported in the transcript instead of silently skipped
<!-- AC:END -->
