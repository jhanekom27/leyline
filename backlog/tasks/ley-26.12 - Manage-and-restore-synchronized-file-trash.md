---
id: LEY-26.12
title: Manage and restore synchronized file trash
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-13 15:40'
updated_date: '2026-10-15 01:19'
labels:
  - files
  - ux
dependencies:
  - LEY-26.7
parent_task_id: LEY-26
type: feature
ordinal: 58000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Make recoverable remote deletes discoverable by listing per-channel sync trash, restoring a chosen file safely, and applying a clear retention/purge policy.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Users can list trashed synchronized files for the active channel
- [x] #2 A trashed file can be restored without overwriting a local file
- [x] #3 Retention and purge behavior are documented and never silently destroy recoverable files
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add safe per-channel sync-trash listing, collision-safe restore, and explicit purge helpers; wire /trash, /trash restore <path>, and /trash purge through background filesystem actions; document indefinite retention and no automatic purge; add focused tests and run the full Rust validation suite.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented /trash, /trash restore <path>, and /trash purge as asynchronous per-channel sync-trash operations. Listing accepts only regular validated entries; restore rejects unsafe paths and never overwrites a local file, using a restored-copy name on collision. Trash has no automatic expiry and purge is explicit. Validation passed: cargo fmt, cargo test (990 passed), cargo clippy (only existing warnings in channel_registry.rs and net.rs), cargo build --release, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added discoverable per-channel sync trash with safe listing, collision-safe restore, and explicit manual purge; documented indefinite retention and verified with focused tests plus the 990-test suite, Clippy, release build, and diff check.
<!-- SECTION:FINAL_SUMMARY:END -->
