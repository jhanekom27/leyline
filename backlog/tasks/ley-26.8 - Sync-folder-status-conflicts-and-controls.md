---
id: LEY-26.8
title: 'Sync folder: status, conflicts, and controls'
status: Done
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 15:08'
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
- [x] #1 A bare `/sync` reports the active channel sync state: on, off, or paused; up to date or how many files are pending; the number of conflicts; last sync time; and the folder path
- [x] #2 `/sync pause` and `/sync resume` stop and restart syncing without losing queued changes
- [x] #3 A channel with sync on shows an indicator in the navigation panel, including when it has errors or conflicts
- [x] #4 Errors such as a full disk, a permission problem, or unreachable members are reported with the file name, and one failing file never stops the others from syncing
- [x] #5 Conflict copies are listed with both file names so they are easy to find and resolve
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Persist off/active/paused sync state; add /sync pause/resume; aggregate runtime pending, completion, error, and conflict state; surface it in /sync and navigation; update documentation and tests without changing transfer mechanics.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LEY-26.3 substrate decision: expose iroh-docs sync and index-application state separately from filesystem-operation status. Surface namespace sync peers/errors and per-file apply/conflict outcomes without blocking the event loop.

Manual two-instance verification passed: /sync reported active/paused state and folder status, pause stopped syncing, resume replayed queued remote work, and sync status/error/conflict indicators were visible. Final validation: cargo fmt, cargo test (962 passed), cargo clippy (existing warnings only), cargo build --release, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added persistent sync pause/resume controls, runtime-derived queue/error/conflict status, detailed /sync reporting, and navigation indicators; verified manually with two instances and automated tests.
<!-- SECTION:FINAL_SUMMARY:END -->
