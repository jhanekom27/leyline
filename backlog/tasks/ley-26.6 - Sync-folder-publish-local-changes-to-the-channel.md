---
id: LEY-26.6
title: 'Sync folder: publish local changes to the channel'
status: Done
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 14:23'
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
- [x] #1 With sync on, a file created or modified in the channel folder is published once it has stopped changing, and other members with sync on receive it
- [x] #2 Files already in the folder when sync is first enabled are published, subject to the size limit
- [x] #3 Hidden files and common temporary or partial files (`.DS_Store`, `*.part`, editor swap files) are ignored
- [x] #4 Changes made while leyline was not running are picked up on the next start
- [x] #5 Files written by sync itself are never published back as local changes
- [x] #6 Publishing never blocks the interface
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add a non-blocking recursive local channel-folder scanner that publishes stable, safe, size-limited files into the existing iroh-docs index; scan existing files at startup/enable; suppress receive-side echoes by resolved index hash; defer deletes and renames. Add focused publisher tests and full validation.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LEY-26.3 substrate decision: publish stable local filesystem changes as writes to the channel iroh-docs namespace using this device docs author. Use the resolved index to suppress sync-originated echoes; do not publish through the chat HashSeq manifest.

Manual verification passed with two running instances joined through a normal group-channel invite: /sync on on both, then a file copied into one channel folder appeared in the other. Initial direct /dm testing exposed separate direct-conversation docs namespaces; captured as follow-up LEY-26.10. Final validation: cargo fmt, cargo test (956 passed), cargo clippy (existing warnings only), cargo build --release, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Published stable local channel-folder files into the replicated index without blocking the UI, with startup scanning, temporary-file filtering, size limits, and remote-write echo suppression. Verified end-to-end in two invited group-channel instances.
<!-- SECTION:FINAL_SUMMARY:END -->
