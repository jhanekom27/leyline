---
id: LEY-16.2
title: 'Opt-in mouse capture: click a channel in the navigation panel to switch'
status: To Do
assignee: []
created_date: '2026-10-12 13:29'
labels:
  - ux
dependencies:
  - LEY-16.1
parent_task_id: LEY-16
type: feature
ordinal: 50000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Clicking a channel or conversation in the left-hand panel needs real mouse reporting, and that has a cost: while a terminal reports the mouse to an app it stops doing native text selection (hold Option in iTerm2, typically Shift elsewhere), and it may intercept Cmd-click on the links from LEY-5, depending on the terminal. For a chat app where people copy text, that should be the choice of the user, so mouse capture is an opt-in toggle, off by default and persisted like `/bell`.

With capture on, terminals report wheel ticks as mouse events instead of arrow keys, so wheel scrolling must keep working the same way as in LEY-16.1. Clicks inside other views (such as the `/files` overlay in LEY-25.7) can build on this later.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/mouse` toggles mouse capture and reports the new state; it is off by default, survives restarts, and the mouse is not captured while it is off
- [ ] #2 With capture on, clicking a channel or conversation in the navigation panel switches to it, including when the panel has scrolled; clicking a section header does nothing
- [ ] #3 Clicks outside the navigation panel, and clicks while an overlay or picker is open, do not switch channels
- [ ] #4 With capture on, the wheel scrolls the transcript exactly as it does in LEY-16.1
- [ ] #5 Mouse capture is switched off when leyline quits, including after a crash, so the shell is not left reporting mouse events
- [ ] #6 The README documents `/mouse` and how to select text while capture is on
<!-- AC:END -->
