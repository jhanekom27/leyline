---
id: LEY-28
title: Refactor core modules and add agent context routing
status: Done
assignee: []
created_date: '2026-10-13 07:16'
updated_date: '2026-10-13 07:27'
labels: []
dependencies: []
priority: high
type: task
ordinal: 55000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Perform a behavior-preserving refactor of the large core Rust modules. Split AppState behavior, rendering, file handling, networking, and runtime orchestration into focused modules while retaining the one-owner-of-UI-state architecture and existing external behavior. Add a concise AGENTS.md entrypoint and topic-routing skills under .agents/skills so coding agents can quickly locate relevant modules and architectural invariants.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Core responsibilities currently concentrated in app.rs, ui.rs, files.rs, net.rs, and main.rs are extracted into focused modules with clear ownership.
- [x] #2 The refactor preserves observable CLI, TUI, networking, persistence, and file-transfer behavior; no feature or protocol changes are introduced.
- [x] #3 All existing tests pass after module moves, with tests kept alongside the modules they cover.
- [x] #4 AGENTS.md is a short repository entrypoint that routes agents to a topic-oriented .agents/skills tree.
- [x] #5 Each skill directs agents to the relevant modules, invariants, and validation commands without duplicating the full repository documentation.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Refactored behavior-preserving responsibilities into focused private modules: app files overlay, network-event state handling, and reply/thread state; UI files overlay rendering; dropped-file parsing; networking protocol helpers; and runtime side-effect orchestration. Added a concise AGENTS.md routing entrypoint, seven topic skills, and three routing evaluation prompts. Validation: cargo fmt --check; cargo test (938 passed); cargo clippy (clean); cargo build --release (passed); skill tree and eval JSON structure validated.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Completed the core-module refactor and agent context harness without changing observable behavior. Module roots remain stable facades while focused child modules isolate core responsibilities; the agent guide now routes work to topic-specific skills.
<!-- SECTION:FINAL_SUMMARY:END -->
