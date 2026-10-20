---
id: LEY-29.8
title: Move keyboard and command reference into the documentation site
status: Done
assignee: []
created_date: '2026-10-20 14:57'
updated_date: '2026-10-20 15:00'
labels:
  - documentation
  - ux
  - commands
dependencies: []
parent_task_id: LEY-29
priority: high
type: docs
ordinal: 88000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Finish the LEY-29 documentation epic by moving the README keyboard-shortcut and slash-command reference into the published mdBook guide. Make the documentation entry point unmistakable from the README, give the guide a documentation-oriented name, and keep contributor-only verification material out of the user-facing project pitch.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A dedicated, navigable guide reference page contains the complete keyboard-shortcut and slash-command tables currently maintained in the README.
- [x] #2 The README removes the duplicated reference tables and exposes an obvious documentation call-to-action near the project introduction, naming the documentation and linking to the published site.
- [x] #3 The guide landing page and navigation make the command and keyboard reference easy to find, and the mdBook title uses documentation-oriented wording.
- [x] #4 The parent LEY-29 acceptance criteria are reviewed and finalized only after this subtask's documentation validation succeeds.
- [x] #5 task docs:build, task docs:test, and git diff --check pass.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add guide/src/reference.md as the authoritative Command & keyboard reference. Move the README Keybindings and Commands tables there without changing command behavior claims; retain their current command variants, safety guidance, and /help framing.
2. Add the reference page to guide/src/SUMMARY.md and link it prominently from the landing page’s Find your way around section. Rename the mdBook title and description in guide/book.toml from guide to documentation so browser and UI metadata matches the user-facing destination.
3. Replace the README’s current low-visibility Documentation section with a prominent early call-to-action immediately after the hero: Read the Leyline documentation, linking to the published site and naming installation, quickstart, commands, and folder sync. Keep the existing visual demos and concise feature narrative. Remove the now-duplicated Keybindings and Commands sections, and remove the contributor-only sync-folder verification from the README because CONTRIBUTING.md already owns it.
4. Validate the generated site and links with task docs:build, task docs:test, and git diff --check. Verify the guide navigation exposes the reference page and that the README remains a short GitHub landing page with the documentation link visible before its feature sections.
5. After this subtask’s validation, review LEY-29’s three parent acceptance criteria against the complete child-task set, record evidence and a final summary in Backlog, mark the subtask and parent Done only when each criterion is objectively satisfied, and leave release/version work untouched.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Moved the complete README keyboard-shortcut and slash-command tables into guide/src/reference.md, preserving every entry except an intentional correction to the /who description for its current you and mine endpoint labels. The guide navigation and landing page link to the reference; mdBook metadata now says Leyline documentation. The README now exposes a prominent Read the Leyline documentation call-to-action immediately after its hero and removes the duplicated tables plus contributor-only sync verification. Validation passed: task docs:build, task docs:test, git diff --check, and an exact migration check against the pre-change README.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Moved the complete keyboard and slash-command reference into the published Leyline documentation, linked it from guide navigation and an early README call-to-action, and removed duplicated README reference material. The parent LEY-29 criteria were reviewed and finalized after validation. Verified with task docs:build, task docs:test, git diff --check, and an exact migration check.
<!-- SECTION:FINAL_SUMMARY:END -->
