---
id: LEY-16
title: Mouse support for the channel panel and scrolling
status: To Do
assignee: []
created_date: '2026-10-08 02:12'
updated_date: '2026-10-12 07:54'
labels:
  - ux
dependencies: []
type: feature
ordinal: 16000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
`crossterm` already supports mouse events, but leyline currently only supports keyboard navigation for switching channels (`Tab`/`Shift+Tab`, `Ctrl+P`) and scrolling the transcript.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Clicking a channel or conversation in the left-hand navigation panel switches to it
- [ ] #2 The scroll wheel scrolls the active channel's transcript, equivalent to Up/Down
<!-- AC:END -->
