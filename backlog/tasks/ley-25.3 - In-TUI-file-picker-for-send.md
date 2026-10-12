---
id: LEY-25.3
title: In-TUI file picker for /send
status: To Do
assignee: []
created_date: '2026-10-12 08:28'
labels:
  - files
  - ux
dependencies:
  - LEY-25.1
parent_task_id: LEY-25
type: feature
ordinal: 36000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
`/send` needs an exact path typed or pasted, which is the biggest friction when sharing from a terminal. A keyboard-driven file browser inside the TUI, in the style of the `Ctrl+P` jump-to overlay, removes that without a GUI dialog, so it also works over SSH and on Linux. Path tab-completion in the message box is related but belongs with LEY-15.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Bare `/send` with no argument opens a modal file browser; `/send <path>` behaves as before
- [ ] #2 Up and Down move the highlight, Enter opens a folder or chooses a file, and Backspace goes up a level; typing narrows the current folder by name, case-insensitively
- [ ] #3 Files are listed newest first so recent downloads and screenshots are on top, with folders listed before files and hidden files left out
- [ ] #4 The browser opens in the folder used last time (remembered across restarts), falling back to the Downloads folder
- [ ] #5 Space marks several files; Enter stages all marked files, or the highlighted one if none are marked
- [ ] #6 Esc closes the browser without staging anything; an unreadable folder shows an error and leaves the browser open
- [ ] #7 Opening a very large or slow folder never freezes input
<!-- AC:END -->
