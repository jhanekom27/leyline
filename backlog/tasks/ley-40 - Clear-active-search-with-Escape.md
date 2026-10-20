---
id: LEY-40
title: Clear active search with Escape
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-19 08:35'
updated_date: '2026-10-20 01:28'
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
- [x] #1 With a search filter active, Escape clears the filter and restores the unfiltered transcript without quitting the application.
- [x] #2 The existing /search or /s command without an argument still clears the active filter.
- [x] #3 Escape keeps its existing behavior when no search filter is active.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add active-search clearing to AppState's normal Escape path before staged attachments, thread closing, and quitting.\n2. Add focused inline tests for Escape clearing an active filter and preserving the no-filter quit behavior; retain the existing /search and /s clearing coverage.\n3. Format and run the Rust test suite, then record results on the task.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented a shared active-search clearing transition for Escape and argument-free /search or /s. Verified with cargo fmt and cargo test (1,057 passed), including search-clear, slash-clear, and normal-Escape regression coverage.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Escape now clears active search filters before other normal fallback actions; /search and /s clearing remain unchanged. Verified by cargo fmt and cargo test (1,057 passed).
<!-- SECTION:FINAL_SUMMARY:END -->
