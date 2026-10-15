---
id: LEY-29.1
title: Document folder sync setup and safe operating model
status: In Progress
assignee: []
created_date: '2026-10-13 15:43'
updated_date: '2026-10-15 04:24'
labels:
  - files
  - ux
dependencies:
  - LEY-26.8
parent_task_id: LEY-29
type: docs
ordinal: 65000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Write user-facing documentation for enabling folder sync, locating channel folders, interpreting status, paused, and conflict indicators, and understanding which channel types currently support sync.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Documentation gives a step-by-step setup path for two invited group-channel instances
- [ ] #2 Documentation explains safety behavior: conflict copies, trash, size limits, pause/resume, and temporary-file exclusions
- [ ] #3 Documentation explicitly distinguishes group-channel support from pending direct-message sync
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Create an mdBook guide in guide/ with a discoverable landing page and a folder-sync chapter covering invited group-channel setup, statuses, safety behavior, and unsupported DM sync. Add a small theme layer, README entry point, generated-output ignore rule, and Taskfile targets to build, preview, configure, and manually publish the static site to gh-pages. The one-time Pages setup uses authenticated gh api requests; do not add a GitHub Actions workflow. Validate the book locally and verify the first manual GitHub Pages publication.
<!-- SECTION:PLAN:END -->
