---
id: LEY-26.7
title: 'Sync folder: propagate deletes and renames'
status: To Do
assignee: []
created_date: '2026-10-12 08:29'
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
- [ ] #1 Deleting a synced file removes it from other members with sync on, and a rename arrives as a rename without transferring the content again
- [ ] #2 A file removed by a remote delete is moved to a per-channel trash folder instead of being deleted outright
- [ ] #3 A file deleted remotely but edited locally since the last sync is kept, not removed
- [ ] #4 A peer that was offline during a delete converges on reconnect without bringing the file back
- [ ] #5 Conflicting concurrent changes (edit versus delete, rename versus rename) never lose data
<!-- AC:END -->
