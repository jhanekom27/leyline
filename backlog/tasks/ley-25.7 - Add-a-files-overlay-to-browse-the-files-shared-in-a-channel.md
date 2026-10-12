---
id: LEY-25.7
title: Add a /files overlay to browse the files shared in a channel
status: To Do
assignee: []
created_date: '2026-10-12 08:28'
labels:
  - files
  - ux
dependencies:
  - LEY-25.6
parent_task_id: LEY-25
type: feature
ordinal: 40000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Once a file message scrolls out of view, finding it again means scrolling the transcript or remembering its hash. The app already indexes every attachment it has seen, so an overlay over that index gives each channel a file browser: a place to save or open files without typing a hash. It follows the `Ctrl+P` jump-to overlay, a modal that stores only its query and highlight.

Clicking an entry could follow if mouse support (LEY-16) is enabled, but the overlay is keyboard-first.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/files` opens a modal overlay listing every file shared in the active channel, including loaded and backfilled history, newest first, with name, size, sender, date, and whether it is saved locally
- [ ] #2 Typing filters by file name or sender, case-insensitively; Up and Down move the highlight; Esc closes the overlay
- [ ] #3 Enter saves the highlighted file the same way `/save` does; a second key opens an already-saved file with the OS default application
- [ ] #4 The saved marker survives restarts and shows a file as unsaved again if it was deleted from disk
- [ ] #5 The overlay works in group channels, `/msg` DMs, and `/dm` chats, and shows a clear message when nothing has been shared
<!-- AC:END -->
