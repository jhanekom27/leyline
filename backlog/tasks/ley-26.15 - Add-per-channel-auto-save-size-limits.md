---
id: LEY-26.15
title: Add per-channel auto-save size limits
status: To Do
assignee: []
created_date: '2026-10-13 15:40'
labels:
  - files
  - trust-safety
dependencies:
  - LEY-26.9
parent_task_id: LEY-26
type: enhancement
ordinal: 61000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Refine the device-wide autosave ceiling with optional per-channel overrides, allowing a generous limit for trusted personal-device channels while retaining a conservative default elsewhere.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A channel can override the device auto-save limit with a finite size
- [ ] #2 A channel without an override uses the device-wide setting
- [ ] #3 Overrides persist per channel and are removed when the channel is left
<!-- AC:END -->
