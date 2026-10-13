---
id: LEY-26.14
title: Add per-channel sync activity history
status: To Do
assignee: []
created_date: '2026-10-13 15:40'
labels:
  - files
  - ux
dependencies:
  - LEY-26.8
parent_task_id: LEY-26
type: feature
ordinal: 60000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Keep a compact, local per-channel record of sync publishes, downloads, deletes, trash moves, conflicts, skips, and failures so activity remains discoverable after transcript notices scroll away.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Users can view recent sync activity for the active channel
- [ ] #2 Activity records file path, action, outcome, and time without exposing secret material
- [ ] #3 History is bounded, persisted locally, and does not affect sync transport
<!-- AC:END -->
