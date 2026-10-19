---
id: LEY-29.7
title: Refresh quickstart after installation and launch paths
status: In Progress
assignee:
  - '@Jurgen'
created_date: '2026-10-15 14:35'
updated_date: '2026-10-19 07:47'
labels:
  - documentation
  - ux
dependencies:
  - LEY-30
  - LEY-31
parent_task_id: LEY-29
type: docs
ordinal: 76000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Refresh the user-facing quickstart after the installation and repository-metadata work lands. Replace the current development-build path with the supported end-user installation flow, verify links and launch-facing wording, and keep the two-peer invite walkthrough accurate.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The quickstart uses the installation path delivered by LEY-30 rather than a development-build-only path
- [ ] #2 Quickstart links and user-facing project metadata reflect the outcomes of LEY-31
- [x] #3 The two-peer invite walkthrough remains concise and technically accurate
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Replace the guide’s development-build entry point with the completed LEY-30 end-user installation path and a launch command for the installed binary.
2. Keep the two-peer invite walkthrough concise, with the separate-home note limited to local testing.
3. Verify internal links and build/test the mdBook guide through the documented local commands.
4. Before finalizing, review repository-metadata links after LEY-31 is complete.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Updated the guide landing page and 60-second walkthrough to install a published release and run `leyline`, rather than clone and build a development checkout. Separate HOME directories now apply only to local two-peer testing. Refreshed the Cargo installation example to v0.21.1.

Validated with `task docs:build` and `task docs:test`.

Acceptance criterion 2 remains pending: LEY-31 is still To Do, so its repository-metadata outcome cannot yet be reflected in the quickstart.
<!-- SECTION:NOTES:END -->
