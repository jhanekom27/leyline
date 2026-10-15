---
id: LEY-26.11
title: Add per-channel sync policies and path filters
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-13 15:40'
updated_date: '2026-10-14 14:19'
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
- [x] #1 A channel can select receive-only or publish-and-receive independently of other channels
- [x] #2 Include and exclude rules apply consistently to startup scans and live changes
- [x] #3 Policy and rules persist across restart and are visible in /sync status
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Replace mode-only sync persistence with version-tolerant per-channel configuration carrying off, receive-only, publish-and-receive, and paused policies plus ordered include/exclude glob rules; migrate legacy active settings to publish-and-receive. Add normalized rule matching at both local publication and remote application boundaries, including paused-event replay. Extend /sync configuration and status reporting, keep UI hints/help/README aligned, add focused persistence, matching, command, and runtime tests, then run formatting, tests, clippy, release build, and diff checks.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented explicit per-channel off, receive-only, publish-and-receive, and paused policies with validated include/exclude glob rules. Legacy active settings migrate to publish-and-receive. The same rules gate local publication and remote application, including replay after pause. Validation passed: cargo fmt, cargo test (984 passed), cargo clippy (only two pre-existing warnings in channel_registry.rs and net.rs), cargo build --release, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added persistent per-channel sync policies and path filters, with /sync configuration and status reporting. Verified by focused persistence and command tests plus the full 984-test suite, Clippy, release build, and diff check.
<!-- SECTION:FINAL_SUMMARY:END -->
