---
id: LEY-16
title: Mouse support for the channel panel and scrolling
status: To Do
assignee: []
created_date: '2026-10-08 02:12'
updated_date: '2026-10-12 13:29'
labels:
  - ux
dependencies: []
type: feature
ordinal: 16000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
`crossterm` already supports mouse events, but leyline currently only supports keyboard navigation for switching channels (`Tab`/`Shift+Tab`, `Ctrl+P`) and scrolling the transcript.

Mouse input is split into two subtasks because the halves have very different costs. Wheel scrolling can use the terminal alternate scroll mode and needs no mouse capture. Clicking needs real mouse reporting, which takes over native text selection in the terminal, so it is an opt-in toggle.
<!-- SECTION:DESCRIPTION:END -->
