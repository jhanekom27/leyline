---
id: LEY-16.1
title: Scroll the transcript with the mouse wheel
status: To Do
assignee: []
created_date: '2026-10-12 13:29'
labels:
  - ux
dependencies: []
parent_task_id: LEY-16
type: feature
ordinal: 49000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Scrolling the transcript is keyboard-only (`Up`/`Down`). Terminals can translate wheel ticks into `Up`/`Down` key presses on the alternate screen (xterm alternate scroll mode, DECSET 1007), which gives wheel scrolling without capturing the mouse, so native text selection and Cmd-click on links keep working. terminfo.dev lists iTerm2, Ghostty, kitty, Terminal.app, Warp, and VS Code as supporting it, and VTE, Konsole, Alacritty, WezTerm, and Windows Terminal do too.

Two existing behaviors matter: the scroll offset is only clamped when rendering, so a long flick past the oldest message would build up distance that must be scrolled back before the view moves; and wheel ticks arrive indistinguishable from arrow keys, so LEY-15 (command history on `Up`) must not turn the wheel into history recall.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Scrolling the mouse wheel in leyline scrolls the active channel transcript like `Up` and `Down` do, verified manually in Warp and one other terminal
- [ ] #2 Native text selection and Cmd-click on links keep working, because the mouse is not captured
- [ ] #3 After scrolling past the oldest message (or past the newest), the first tick back the other way moves the view, with no hidden scroll distance to work off first
- [ ] #4 The terminal mode is restored when leyline quits
<!-- AC:END -->
