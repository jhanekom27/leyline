---
id: LEY-42
title: Split app and UI modules into focused components
status: To Do
assignee: []
created_date: '2026-10-19 13:25'
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
- [ ] #1 src/app.rs is reduced to a clear AppState-facing facade while cohesive state-transition responsibilities move into focused src/app/ modules.
- [ ] #2 src/ui.rs is reduced to a clear rendering facade while cohesive rendering responsibilities move into focused src/ui/ modules.
- [ ] #3 The refactor preserves the architecture boundaries: app modules perform no network or filesystem I/O, and UI modules only read AppState without mutation or I/O.
- [ ] #4 Existing behavior remains unchanged and cargo fmt, cargo test, cargo clippy, and cargo build --release pass.
<!-- AC:END -->
