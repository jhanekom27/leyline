---
id: LEY-26.17
title: Use readable stable folders for point-to-point sync
status: Done
assignee: []
created_date: '2026-10-14 06:12'
updated_date: '2026-10-14 10:05'
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
- [x] #1 A new point-to-point conversation receives a readable local sync-folder name while retaining a stable collision-resistant identity suffix
- [x] #2 Existing direct conversations keep their current p2p folder without moving or duplicating files
- [x] #3 Sync publishing, remote apply, saves, conflicts, and /sync status consistently resolve the same mapped direct folder
- [x] #4 Folder names remain stable when aliases or nicknames later change
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Extend DirectRegistry with an optional local folder mapping, preserving existing p2p-* folders when loading older records. 2. Generate a readable, collision-resistant folder name for new direct conversations from the current display name plus the peer-id suffix; persist it once. 3. Add one runtime resolver and use it for direct publish scans, remote apply/trash/relocation, saves, and /sync status while leaving group paths unchanged. 4. Add migration, stability, collision, and path-resolution tests; run cargo fmt, cargo test, cargo clippy, and cargo build --release.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation verification: DirectRegistry now persists an optional local folder mapping with backward-compatible peer-only and pre-folder decoders. New direct peers receive a safe dm-<label>-<id> folder; old records remain unmapped and continue using p2p-* paths. AppState resolves the mapping for saves, autosave, /folder, /sync, outbox, publish scanning, remote apply, conflicts, and deletion. Tests cover name generation, record migration/persistence, mapped saves, alias stability, and sync status.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
New direct conversations now use readable, stable local folders such as dm-matt-48433c9d while p2p channel IDs remain unchanged for routing and state. Existing conversations retain their p2p-* folders without migration. The mapping is persisted and does not follow later alias/nickname changes. Verified with cargo fmt, 972 passing tests, cargo clippy, cargo build --release, and git diff --check.
<!-- SECTION:FINAL_SUMMARY:END -->
