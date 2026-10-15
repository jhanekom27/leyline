---
id: LEY-26.14
title: Add per-channel sync activity history
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-13 15:40'
updated_date: '2026-10-15 02:12'
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
- [x] #1 Users can view recent sync activity for the active channel
- [x] #2 Activity records file path, action, outcome, and time without exposing secret material
- [x] #3 History is bounded, persisted locally, and does not affect sync transport
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add bounded postcard-backed per-channel sync activity persistence; record runtime publish, download, delete, trash, conflict, skip, and failure outcomes without affecting transport; add an /activity active-channel view; update documentation and tests; validate persistence, bounds, and transport isolation.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added bounded local postcard-backed per-channel sync activity history and /activity. Runtime records publish/delete queueing, remote applies, skips, conflicts, trash moves, and scan failures without changing transport. Repeated queued publishes are coalesced by channel and path. Validation passed: cargo fmt, cargo test (997 passed), cargo clippy (only existing warnings in channel_registry.rs and net.rs), cargo build --release, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added bounded persisted per-channel sync activity with an /activity view and deduplicated queued publish records. Verified with focused persistence/deduplication tests and the 997-test suite, Clippy, release build, and diff check.
<!-- SECTION:FINAL_SUMMARY:END -->
