---
id: LEY-26.7
title: 'Sync folder: propagate deletes and renames'
status: Done
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 14:47'
labels:
  - files
  - networking
dependencies:
  - LEY-26.5
  - LEY-26.6
parent_task_id: LEY-26
type: feature
ordinal: 47000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provisional until the spike lands. Without deletes, stale files pile up, and without renames every rename becomes a copy. Deleting is also how one member could wipe the shared folder for everyone by accident, so propagation needs to be recoverable.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Deleting a synced file removes it from other members with sync on, and a rename arrives as a rename without transferring the content again
- [x] #2 A file removed by a remote delete is moved to a per-channel trash folder instead of being deleted outright
- [x] #3 A file deleted remotely but edited locally since the last sync is kept, not removed
- [x] #4 A peer that was offline during a delete converges on reconnect without bringing the file back
- [x] #5 Conflicting concurrent changes (edit versus delete, rename versus rename) never lose data
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Publish stable local absences as iroh-docs tombstones; represent renames as new-path publication plus old-path tombstone; decode remote tombstones; safely move matching remote deletes to per-channel trash; preserve local edits and concurrent conflicts without data loss; add offline/live tests and validate fully.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LEY-26.3 substrate decision: represent deletes with iroh-docs tombstones. Model a rename as an add at the new path plus a delete at the old path, retaining conflict and trash policy in this task rather than in the substrate layer.

Manual two-instance verification passed: delete propagation moved remote files into per-channel trash, rename arrived without data loss, offline deletion converged after reconnect, and local-edit-versus-remote-delete retained the local file. Final validation: cargo fmt, cargo test (961 passed), cargo clippy (existing warnings only), cargo build --release, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented recoverable delete and rename sync with docs tombstones, per-channel trash, offline convergence, and lossless conflict preservation; verified manually with two instances and automated sync tests.
<!-- SECTION:FINAL_SUMMARY:END -->
