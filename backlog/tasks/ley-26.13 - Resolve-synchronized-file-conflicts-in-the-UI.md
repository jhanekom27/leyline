---
id: LEY-26.13
title: Resolve synchronized file conflicts in the UI
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-13 15:40'
updated_date: '2026-10-15 01:34'
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
- [x] #1 Users can list unresolved sync conflicts with local and remote paths, timestamps, and authors when known
- [x] #2 Resolution actions preserve data and never overwrite a conflicting copy silently
- [x] #3 Navigation and /sync status clear the conflict indicator only after resolution
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Replace path-pair sync conflicts with structured local/remote provenance; add a focused /conflicts transcript view with explicit local, remote, and both resolution actions; perform safe recoverable filesystem moves on the blocking pool; clear conflict status only after success; update documentation and tests, then run the full Rust validation suite.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented structured file and remote-delete conflict records with paths, known modification timestamps, remote author metadata when available, and recoverable remote-copy paths. /conflicts lists and resolves active-channel conflicts with local, remote, or both actions. Displaced copies move to sync trash, and status clears only after the background filesystem resolution succeeds. Validation passed: cargo fmt, cargo test (994 passed), cargo clippy (only existing warnings in channel_registry.rs and net.rs), cargo build --release, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added focused conflict listing and safe local, remote, or both resolution actions with recoverable moves to sync trash. Verified provenance reporting, data-preserving resolution, and status lifecycle with focused tests plus the 994-test suite, Clippy, release build, and diff check.
<!-- SECTION:FINAL_SUMMARY:END -->
