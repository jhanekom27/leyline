---
id: LEY-26.11
title: Add per-channel sync policies and path filters
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
ordinal: 57000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Let each channel choose receive-only, publish-and-receive, paused, or off behavior, and limit sync to configured include/exclude path rules. This refines the current all-or-nothing device-local /sync setting without changing transfer integrity.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A channel can select receive-only or publish-and-receive independently of other channels
- [ ] #2 Include and exclude rules apply consistently to startup scans and live changes
- [ ] #3 Policy and rules persist across restart and are visible in /sync status
<!-- AC:END -->
