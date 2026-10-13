---
id: LEY-26.6
title: 'Sync folder: publish local changes to the channel'
status: To Do
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 08:27'
labels:
  - files
  - networking
dependencies:
  - LEY-26.5
parent_task_id: LEY-26
type: feature
ordinal: 46000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provisional until the spike lands. The send half of two-way sync: files created or edited in the channel folder of an opted-in device are added to the channel file index, so other members receive them.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 With sync on, a file created or modified in the channel folder is published once it has stopped changing, and other members with sync on receive it
- [ ] #2 Files already in the folder when sync is first enabled are published, subject to the size limit
- [ ] #3 Hidden files and common temporary or partial files (`.DS_Store`, `*.part`, editor swap files) are ignored
- [ ] #4 Changes made while leyline was not running are picked up on the next start
- [ ] #5 Files written by sync itself are never published back as local changes
- [ ] #6 Publishing never blocks the interface
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LEY-26.3 substrate decision: publish stable local filesystem changes as writes to the channel iroh-docs namespace using this device docs author. Use the resolved index to suppress sync-originated echoes; do not publish through the chat HashSeq manifest.
<!-- SECTION:NOTES:END -->
