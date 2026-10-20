---
id: LEY-41
title: Design command-picker behavior for argument-taking commands
status: Done
assignee: []
created_date: '2026-10-19 08:35'
updated_date: '2026-10-20 03:38'
labels:
  - ux
  - commands
dependencies: []
priority: medium
type: spike
ordinal: 84000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Define a consistent Ctrl+K picker interaction for commands that need typed arguments, optional arguments, or no arguments, building on LEY-33 before implementation changes are selected.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The task records a recommended interaction for no-argument, required-argument, and optional-argument commands, including /files and /search examples.
- [x] #2 The recommendation specifies when selecting an item executes immediately versus inserts command text into the compose input.
- [x] #3 The recommendation identifies any needed picker presentation changes and focused tests for a follow-on implementation task.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Design decision (2026-10-20): add an explicit picker-selection policy to the shared command catalog instead of deriving behavior from bracket syntax.

- Execute: every no-argument command, including /files; and optional-argument commands with a safe, useful bare default: /send (file picker), /folder, /maxsize, /autosave, /outbox, /sync, /conflicts, /trash, and /reply.
- Insert: every command with required arguments, including /search, which inserts `/search `; plus optional commands whose bare default is surprising or mutating: /pair, /save, /leave, and /rename. /pair inserts `/pair ` so ticket creation or redemption remains an explicit user choice. /leave inserts bare `/leave`; the user must press Enter again to invoke its existing leave behavior, making selection a deliberate confirmation step.
- Insertion replaces the active unsent draft and places the cursor at the end of the canonical command input. Execution keeps the existing draft.

Picker presentation: preserve the compact command/shortcut/description columns. Change the footer according to the highlighted command: `Enter run · draft kept` for Execute and `Enter insert command · replaces draft` for Insert.

Follow-on focused coverage: app-state tests for /files execution, /search insertion and draft replacement, a safe optional command such as /send, and /pair, /save, /leave, and /rename insertion; preserve filtering, ranking, clamping, and Esc coverage. Add renderer tests for outcome-specific footer copy and narrow terminals. Update /help and README so they describe the run-or-insert behavior and draft-replacement warning.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Defined and recorded the Ctrl+K command-selection policy. Verified the completed design explicitly covers /files, /search, required and optional arguments, immediate execution versus draft insertion, draft handling, picker footer feedback, documentation updates, and focused follow-on tests.
<!-- SECTION:FINAL_SUMMARY:END -->
