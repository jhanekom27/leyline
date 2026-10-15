---
id: LEY-26
title: Per-channel shared folders
status: Done
assignee: []
created_date: '2026-10-12 08:28'
updated_date: '2026-10-15 02:50'
labels:
  - files
  - networking
dependencies: []
type: feature
ordinal: 33000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Moving a file between machines owned by the same person (for example macOS to Linux, where it currently means emailing it to yourself), or sharing a few small files with friends, should not need a command per file. A folder per channel on disk that keeps itself in sync removes that friction.

The work is staged: opt-in auto-save (receive only), a watched outbox (send only), then true two-way sync once a spike has chosen the substrate. Access follows the existing capability model (decision-2): anyone holding the channel ticket is a member, so there is no extra invite step. A private channel shared by your own paired devices (LEY-20) already covers the personal cross-device case and needs no separate feature. The two-way sync subtasks are provisional until the spike lands.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
All 17 child tasks are complete. LEY-26.2's final manual outbox checks confirmed stable attachment delivery, initial-folder snapshotting, temporary-file filtering, durable deduplication, and transcript reporting for oversized files.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Completed per-channel shared folders through all child tasks: opt-in auto-save, outbox sharing, two-way sync, policies, conflict/trash management, activity history, and per-channel auto-save limits.
<!-- SECTION:FINAL_SUMMARY:END -->
