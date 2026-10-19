---
id: LEY-40
title: Clear active search with Escape
status: To Do
assignee: []
created_date: '2026-10-19 08:35'
labels:
  - ux
  - search
dependencies: []
priority: medium
type: bug
ordinal: 83000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Make Escape leave an active /search view before it performs the normal application-level escape or quit action.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 With a search filter active, Escape clears the filter and restores the unfiltered transcript without quitting the application.
- [ ] #2 The existing /search or /s command without an argument still clears the active filter.
- [ ] #3 Escape keeps its existing behavior when no search filter is active.
<!-- AC:END -->
