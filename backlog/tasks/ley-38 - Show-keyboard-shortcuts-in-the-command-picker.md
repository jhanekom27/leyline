---
id: LEY-38
title: Show keyboard shortcuts in the command picker
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-19 08:35'
updated_date: '2026-10-19 14:01'
labels:
  - ux
  - commands
dependencies: []
priority: medium
type: enhancement
ordinal: 81000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend the Ctrl+K command picker introduced by LEY-33 so users can discover keyboard shortcuts that invoke the same command without leaving the picker.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The Ctrl+K picker shows the keyboard shortcut for every command that has one.
- [x] #2 Commands without a keyboard shortcut remain clearly distinguishable and do not show misleading placeholder text.
- [x] #3 Filtering, ranking, selection, and readable narrow-terminal layout continue to work.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Correct the command shortcut inventory to include Ctrl+R for /reply and Ctrl+T for /thread, alongside Ctrl+V for /paste. 2. Present Ctrl+K in the picker title and Ctrl+P channel search in its controls. 3. Replace inline labels with fixed command, shortcut, and description columns, leaving unbound shortcut cells visibly empty. 4. Extend app and renderer tests for shortcut coverage, column alignment, filtering, and narrow terminals; rerun validation.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added optional shortcut metadata to the shared command catalog and marked /paste as Ctrl+V, the only global binding that executes the same command path. The picker renders an optional [Ctrl+V] label with no placeholder for unbound commands and clamps each span for narrow terminals. Validation: cargo fmt; cargo test command_picker (8 passed); cargo test (1,054 passed after one unrelated sync-index peer timing retry); git diff --check.

Additional cross-module validation: cargo clippy and cargo build --release both succeeded. Clippy emitted three pre-existing warnings (unused channel registry methods, an existing command-picker ? suggestion, and net::start argument count); release emitted the unused-method warning.

Reopened after review: Ctrl+R reply selection, Ctrl+T thread selection, Ctrl+P channel search, and Ctrl+K opening the picker must be discoverable; shortcut labels will move into a dedicated column.

Correction complete: the catalog now exposes Ctrl+V for /paste, Ctrl+R for /reply's target picker, and Ctrl+T for /thread's target picker. Ctrl+K is shown in the picker title and Ctrl+P channel search in its controls. Rows now use stable command, shortcut, and description columns with a header; commands with no shortcut intentionally have an empty shortcut cell. Validation: cargo fmt; cargo test command_picker (9 passed); cargo test (1,055 passed); cargo clippy; cargo build --release; git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Corrected Ctrl+K shortcut discovery with Ctrl+R reply, Ctrl+T thread, Ctrl+P channel search, and Ctrl+K picker guidance, plus a dedicated shortcut column. Verified with focused renderer coverage and the full 1,055-test suite.
<!-- SECTION:FINAL_SUMMARY:END -->
