---
id: LEY-26.8
title: 'Sync folder: status, conflicts, and controls'
status: To Do
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 08:27'
labels:
  - files
  - ux
dependencies:
  - LEY-26.5
  - LEY-26.6
parent_task_id: LEY-26
type: feature
ordinal: 48000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provisional until the spike lands. Sync that fails silently is worse than no sync, so there must be a way to see whether a channel folder is up to date, what is pending or in conflict, and to pause it.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A bare `/sync` reports the active channel sync state: on, off, or paused; up to date or how many files are pending; the number of conflicts; last sync time; and the folder path
- [ ] #2 `/sync pause` and `/sync resume` stop and restart syncing without losing queued changes
- [ ] #3 A channel with sync on shows an indicator in the navigation panel, including when it has errors or conflicts
- [ ] #4 Errors such as a full disk, a permission problem, or unreachable members are reported with the file name, and one failing file never stops the others from syncing
- [ ] #5 Conflict copies are listed with both file names so they are easy to find and resolve
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LEY-26.3 substrate decision: expose iroh-docs sync and index-application state separately from filesystem-operation status. Surface namespace sync peers/errors and per-file apply/conflict outcomes without blocking the event loop.
<!-- SECTION:NOTES:END -->
