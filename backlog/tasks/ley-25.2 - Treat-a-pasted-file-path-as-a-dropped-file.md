---
id: LEY-25.2
title: Treat a pasted file path as a dropped file
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
ordinal: 35000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Dragging a file onto the terminal makes the terminal paste its path, and leyline inserts that into the message box as plain text, so the sender still has to type `/send ` in front by hand. Bracketed paste is already enabled, and `/send` already strips the quotes some drag-and-drop sources add. Terminals report only the path, never where the file was dropped, so the target is always the active channel.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A bracketed paste made up only of existing file paths (backslash-escaped, quoted, or `file://` URIs; separated by spaces or newlines) stages those files instead of inserting text
- [ ] #2 A paste that is not entirely existing paths, including ordinary text that merely mentions a path, is inserted into the message box unchanged
- [ ] #3 Several dropped files are staged together; any path that is missing or unreadable is reported and skipped
- [ ] #4 Dropping a file onto leyline works in Warp and in at least one other terminal on macOS or Linux (verified manually)
<!-- AC:END -->
