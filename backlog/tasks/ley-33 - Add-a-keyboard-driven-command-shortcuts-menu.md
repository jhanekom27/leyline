---
id: LEY-33
title: Add a keyboard-driven command shortcuts menu
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-16 13:17'
updated_date: '2026-10-16 23:17'
labels: []
dependencies: []
priority: medium
type: feature
ordinal: 77000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provide a fast, discoverable command picker inspired by Omarchy’s Super+K menu. It should let people browse and select available slash commands without first knowing their names, offering a quicker route than scanning the current hints menu.

Each command entry should include a short description that explains its purpose without requiring people to open documentation. The interaction and command coverage should align with existing command parsing, help, and UI conventions.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A user can open a dedicated keyboard-driven menu that lists the available slash commands.
- [x] #2 The menu supports fast navigation or filtering so a command can be found without already knowing its exact name.
- [x] #3 Every listed command presents a concise, user-facing description of what it does.
- [x] #4 Selecting an item follows the established command-entry behavior (for example, inserting or invoking the chosen command as appropriate).
- [x] #5 The feature is covered by focused tests and user-facing command/help documentation is kept consistent where applicable.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Define one app-owned command catalog with canonical syntax, aliases, and concise descriptions; use it to align picker data with parser, hints, help, and README coverage.
2. Add a CommandPicker modal to AppState, opened by Ctrl+K, with case-insensitive filtering across syntax, aliases, and descriptions. Rank syntax and alias matches ahead of description-only matches, with clamped selection and dedicated input ownership.
3. On Enter, close the picker and dispatch the selected slash command through the existing command path, returning its InputAction to the event loop while retaining any prior draft.
4. Render a cleared top-anchored overlay with a filter, syntax/description rows, readable selected-row contrast, 18 visible rows before scrolling, and tiny-terminal safety.
5. Add focused app and renderer tests; update README and /help; run cargo fmt and cargo test.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented a catalog-backed Ctrl+K command picker. It filters syntax, aliases, and descriptions; Enter replaces the active draft without invoking it. Added app-state and renderer coverage; updated /help and README. Validation: cargo fmt --check, cargo test (1010 passed), cargo clippy --all-targets, and cargo build --release.

Follow-up: enlarging the command picker and correcting selected-row contrast after UI feedback.

Follow-up complete: command picker now shows up to 18 rows before scrolling, and selected syntax and description text explicitly render black on cyan for contrast. Validation: cargo fmt, cargo test command_picker (5 passed), and cargo test (1011 passed).

Follow-up: rank command-name and alias matches above description-only matches; make Enter execute the selected command.

Follow-up complete: syntax and alias matches now rank ahead of description-only matches, and Enter dispatches the selected command through the normal command path while retaining the draft. Validation: cargo fmt, cargo test command_picker (5 passed), and cargo test (1011 passed).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added the Ctrl+K slash-command picker with catalog-driven hints, ranked search, high-contrast rows, and direct command execution. Verified by 1,011 passing tests.
<!-- SECTION:FINAL_SUMMARY:END -->
