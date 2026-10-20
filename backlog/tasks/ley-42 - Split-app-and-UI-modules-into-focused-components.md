---
id: LEY-42
title: Split app and UI modules into focused components
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-19 13:25'
updated_date: '2026-10-20 05:10'
labels:
  - refactor
  - architecture
dependencies: []
modified_files:
  - src/app.rs
  - src/ui.rs
priority: medium
type: chore
ordinal: 85000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Follow up on LEY-28 by decomposing the still-oversized src/app.rs and src/ui.rs into cohesive modules without changing user-visible behavior. Preserve leyline’s event-loop architecture: app state transitions remain pure, and rendering remains read-only.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 src/app.rs is reduced to a clear AppState-facing facade while cohesive state-transition responsibilities move into focused src/app/ modules.
- [x] #2 src/ui.rs is reduced to a clear rendering facade while cohesive rendering responsibilities move into focused src/ui/ modules.
- [x] #3 The refactor preserves the architecture boundaries: app modules perform no network or filesystem I/O, and UI modules only read AppState without mutation or I/O.
- [x] #4 Existing behavior remains unchanged and cargo fmt, cargo test, cargo clippy, and cargo build --release pass.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Keep src/app.rs and src/ui.rs as stable facades while extracting focused private responsibility modules without changing behavior or architecture boundaries. 2. Relocate the facade regression suites to src/app/tests.rs and src/ui/tests.rs while retaining assertions. 3. Update app-state, UI-rendering, architecture, and testing skills with new module ownership. 4. Verify with cargo fmt, cargo test, cargo clippy, and cargo build --release.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Extracted AppState responsibilities into focused private app modules and UI rendering concerns into focused private UI modules; roots now remain facades. Updated agent routing and testing skills. Validation passed: cargo fmt --check; cargo test (1,070 passed); cargo clippy (only pre-existing channel_registry dead-code and net::start argument-count warnings); cargo build --release.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Split app and UI facades into focused private modules without behavior changes, and updated agent routing skills. Verified by cargo fmt --check, 1,070 passing tests, cargo clippy, and a successful release build.
<!-- SECTION:FINAL_SUMMARY:END -->
