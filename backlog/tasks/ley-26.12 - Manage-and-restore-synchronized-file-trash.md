---
id: LEY-26.12
title: Manage and restore synchronized file trash
status: To Do
assignee: []
created_date: '2026-10-13 15:40'
labels:
  - files
  - ux
dependencies:
  - LEY-26.7
parent_task_id: LEY-26
type: feature
ordinal: 58000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Make recoverable remote deletes discoverable by listing per-channel sync trash, restoring a chosen file safely, and applying a clear retention/purge policy.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Users can list trashed synchronized files for the active channel
- [ ] #2 A trashed file can be restored without overwriting a local file
- [ ] #3 Retention and purge behavior are documented and never silently destroy recoverable files
<!-- AC:END -->
