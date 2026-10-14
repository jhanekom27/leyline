---
id: LEY-26.17
title: Use readable stable folders for point-to-point sync
status: To Do
assignee: []
created_date: '2026-10-14 06:12'
updated_date: '2026-10-14 06:12'
labels:
  - files
  - app-state
dependencies:
  - LEY-26.10
modified_files:
  - src/direct_registry.rs
  - src/files.rs
  - src/main.rs
  - src/app.rs
parent_task_id: LEY-26
type: enhancement
ordinal: 68000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Point-to-point sync currently stores files under opaque p2p-<id-prefix> folders. Preserve the immutable direct channel identifier for networking and state, but persist a distinct, readable local folder mapping for new direct conversations.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A new point-to-point conversation receives a readable local sync-folder name while retaining a stable collision-resistant identity suffix
- [ ] #2 Existing direct conversations keep their current p2p folder without moving or duplicating files
- [ ] #3 Sync publishing, remote apply, saves, conflicts, and /sync status consistently resolve the same mapped direct folder
- [ ] #4 Folder names remain stable when aliases or nicknames later change
<!-- AC:END -->
