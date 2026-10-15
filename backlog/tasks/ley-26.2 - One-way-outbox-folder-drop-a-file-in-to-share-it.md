---
id: LEY-26.2
title: 'One-way outbox folder: drop a file in to share it'
status: Done
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-15 02:50'
labels:
  - files
  - ux
dependencies:
  - LEY-25.6
parent_task_id: LEY-26
type: feature
ordinal: 42000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Sharing a file from the file manager currently means switching to leyline and picking or pasting it. A watched `send/` folder per channel turns sharing into a drag in Finder or a `cp` on Linux. It is deliberately one-way and not synced, so there are no conflicts or echo loops: each file becomes an ordinary attachment message, and history, backfill, and replies work unchanged. Auto-save covers the receiving side.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 `/outbox on` and `/outbox off` switch the outbox for the active channel; it is off by default, persists across restarts, and enabling it creates `<channel folder>/send/` and reports its location
- [x] #2 A new file appearing in `send/` is shared with the channel as an attachment once it has stopped changing, so a file still being copied is never sent half-written
- [x] #3 Files already in `send/` when the outbox is first enabled are treated as already shared and are not sent
- [x] #4 Hidden files and common temporary or partial files (such as `.DS_Store`, `*.part`, `*.crdownload`, `~$*`) are ignored
- [x] #5 A file is never shared twice, including after a restart; editing it after it was shared does not send it again
- [x] #6 Files over the `/send` size limit, or unreadable ones, are reported in the transcript instead of silently skipped
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Implement a persisted per-channel outbox configuration and pure /outbox command actions. Route folder creation, asynchronous polling, stable-file detection, filtering, validation, deduplication, and transcript errors through the runtime while reusing the existing outbound send path. Add focused persistence, command, and watcher tests; then run cargo fmt, cargo test, cargo clippy, and cargo build --release.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented persistent per-channel outboxes at <channel folder>/send/ with /outbox on and /outbox off. The runtime polls folders on the blocking pool, waits for a second unchanged scan, ignores hidden and temporary names, persists observed names before sending, and routes validation errors to the owning transcript. Verified with cargo fmt, cargo test (944 passed), cargo clippy, cargo build --release, and git diff --check.

Manual verification: an enabled outbox skipped /tmp/leyline-outbox-over-limit.bin (501 MiB, above the default 500 MiB limit) and reported the size-limit reason in the owning channel transcript.

Manual verification: confirmed stable new outbox files become attachment messages only after settling; files present at enable time are not sent; hidden and temporary files are ignored; and a shared file is not re-sent after restart or edit.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added one-way per-channel send folders with persistent setup, stable-file detection, durable de-duplication, validation feedback, documentation, and focused tests.
<!-- SECTION:FINAL_SUMMARY:END -->
