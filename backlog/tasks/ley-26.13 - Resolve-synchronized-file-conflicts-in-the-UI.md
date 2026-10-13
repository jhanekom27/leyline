---
id: LEY-26.13
title: Resolve synchronized file conflicts in the UI
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
ordinal: 59000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provide a focused conflict view and resolution actions for files preserved during concurrent edits, deletes, or renames.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Users can list unresolved sync conflicts with local and remote paths, timestamps, and authors when known
- [ ] #2 Resolution actions preserve data and never overwrite a conflicting copy silently
- [ ] #3 Navigation and /sync status clear the conflict indicator only after resolution
<!-- AC:END -->
