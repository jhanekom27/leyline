---
id: LEY-26.5
title: 'Sync folder: apply remote files to the local channel folder'
status: In Progress
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 12:05'
labels:
  - files
  - networking
dependencies:
  - LEY-26.4
  - LEY-25.6
  - LEY-25.5
parent_task_id: LEY-26
type: feature
ordinal: 45000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provisional until the spike lands. The receive half of two-way sync and its first user-visible slice: a device that opts in mirrors the channel file index into its channel folder. Because other channel members control what appears there, safety comes before speed. The target is a few small files moving between machines such as macOS and Linux, and friends sharing a few files, not bulk backup.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/sync on` and `/sync off` switch mirroring for the active channel on this device; it is off by default and the setting persists
- [ ] #2 With sync on, files added or changed by other members appear in the channel folder under their relative paths, are never visible half-written, and can be fetched from any available member
- [ ] #3 Paths received from peers are validated: absolute paths, `..` components, symlinks, and unsafe names are rejected without touching anything outside the channel folder
- [ ] #4 A remote change never overwrites a local file that has unsynced local edits; the remote version is kept as a clearly named conflict copy instead
- [ ] #5 Names that would collide only because of case (case-insensitive macOS versus case-sensitive Linux) never cause either file to be lost
- [ ] #6 Files over the size limit are skipped with a notice
- [ ] #7 Verified manually between a macOS and a Linux machine
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add persistent per-channel /sync receive settings, emit typed remote index entries, validate relative paths and filesystem safety, then atomically fetch verified blobs into conflict-safe channel destinations. Add unit and two-peer tests plus macOS/Linux manual verification guidance without implementing local publication or deletes/renames.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LEY-26.3 substrate decision: consume the resolved iroh-docs per-channel index from LEY-26.4 as the remote source of truth. Apply only live docs events or reconciled entries through the runtime; do not reimplement history manifests or file transfer, because iroh-docs uses the shared blob store.
<!-- SECTION:NOTES:END -->
