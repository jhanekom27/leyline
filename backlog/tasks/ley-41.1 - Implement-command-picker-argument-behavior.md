---
id: LEY-41.1
title: Implement command-picker argument behavior
status: Done
assignee: []
created_date: '2026-10-20 04:15'
updated_date: '2026-10-20 04:20'
labels:
  - ux
  - commands
dependencies: []
parent_task_id: LEY-41
priority: medium
type: enhancement
ordinal: 86000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Implement the finalized LEY-41 Ctrl+K interaction policy. Add explicit per-command execute versus insert metadata; preserve direct command behavior; render a visible action indicator and outcome-specific footer; align command-picker guidance and focused tests.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 /files and other execute-policy entries run immediately while retaining an existing draft; required-input entries such as /search insert canonical command text without running.
- [x] #2 The finalized optional-command policy is represented: safe bare defaults execute, while /pair, /save, /leave, and /rename insert command text; insertion replaces the active draft and positions the cursor at the end.
- [x] #3 The picker displays a distinct per-row action indicator for execute versus insert and an outcome-specific footer without breaking narrow-terminal rendering.
- [x] #4 In-app help, README, focused app/UI tests, formatting, full test suite, clippy, and release build validate the behavior.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented the catalog-backed Ctrl+K policy: Execute entries retain the draft and dispatch through the existing command path; Insert entries replace the draft with canonical command text and do not dispatch.

The picker now has an action column (`▶ run` or `✎ edit`) and outcome-specific footer guidance. `/help` and README describe the same behavior.

Validation: cargo fmt; cargo test command_picker (13 passed); cargo test (1,061 passed); cargo clippy; cargo build --release; git diff --check. Clippy and the release build report only pre-existing warnings for unused ChannelRegistry methods and Net::start argument count.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented the finalized Ctrl+K execute-or-insert policy with catalog metadata, action icons, outcome-specific footer guidance, aligned help and README, and focused app/UI coverage. Verified with cargo fmt, 13 focused picker tests, 1,061 full tests, cargo clippy, cargo build --release, and git diff --check.
<!-- SECTION:FINAL_SUMMARY:END -->
