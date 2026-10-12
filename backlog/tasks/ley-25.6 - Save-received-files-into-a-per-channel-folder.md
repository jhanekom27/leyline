---
id: LEY-25.6
title: Save received files into a per-channel folder
status: To Do
assignee: []
created_date: '2026-10-12 08:28'
updated_date: '2026-10-12 13:29'
labels:
  - files
  - ux
dependencies: []
parent_task_id: LEY-25
type: enhancement
ordinal: 39000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Every channel saves into one flat `~/Downloads/leyline/`, so files from different channels and people mix together and collide into `name (1).ext`. A stable folder per channel under a single files root is also the groundwork that the `/files` overlay, auto-save, the outbox, and synced folders all need.

Decision: the files root stays in Downloads (`~/Downloads/leyline`, as today). Only the per-channel subfolder is new.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/save` writes into `<files root>/<channel folder>/`, creating it on demand; filename sanitization and the `name (1).ext` collision handling are unchanged
- [ ] #2 A channel folder name comes from the channel stored name, not its display label, so `/rename` and `/alias` never move it
- [ ] #3 Channel folder names are safe on every platform, and two different channels never share a folder
- [ ] #4 The files root defaults to `~/Downloads/leyline` as today, is a persisted setting, and a command shows where it is
<!-- AC:END -->
