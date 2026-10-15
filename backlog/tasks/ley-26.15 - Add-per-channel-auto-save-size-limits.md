---
id: LEY-26.15
title: Add per-channel auto-save size limits
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-13 15:40'
updated_date: '2026-10-15 02:34'
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
- [x] #1 A channel can override the device auto-save limit with a finite size
- [x] #2 A channel without an override uses the device-wide setting
- [x] #3 Overrides persist per channel and are removed when the channel is left
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Persist optional finite per-channel auto-save limits with legacy migration; extend /autosave to set, clear, and report the active-channel override; fall back to the device-wide limit; clear overrides on leave; update docs/tests and validate fully.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented persisted finite per-channel auto-save limit overrides with migration from legacy auto-save settings. /autosave limit <size> applies to the active channel, default restores its device-wide fallback, and leave/disable removes the override. Validation passed: cargo fmt, cargo test (999 passed), cargo clippy (only existing warnings in channel_registry.rs and net.rs), cargo build --release, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added persisted per-channel auto-save limit overrides with device-wide fallback and leave cleanup. Verified by focused override persistence/fallback/cleanup tests and the 999-test suite, Clippy, release build, and diff check.
<!-- SECTION:FINAL_SUMMARY:END -->
