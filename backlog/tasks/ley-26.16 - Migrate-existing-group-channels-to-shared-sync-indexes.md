---
id: LEY-26.16
title: Migrate existing group channels to shared sync indexes
status: Done
assignee: []
created_date: '2026-10-14 06:12'
updated_date: '2026-10-14 09:56'
labels:
  - networking
  - files
dependencies:
  - LEY-26.4
modified_files:
  - src/app/net_events.rs
  - src/main.rs
  - src/message.rs
  - src/net.rs
  - src/sync_index.rs
parent_task_id: LEY-26
type: bug
ordinal: 67000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Existing group channels created before iroh-docs capability propagation can exchange gossip messages while each device independently creates a separate sync index. Reconcile one shared capability over the existing authenticated group channel so legacy members can synchronize without an additional invite.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Two existing members of a ticket-backed group channel that predate docs capabilities converge on one persisted docs namespace without a new invite
- [x] #2 A stable file synced after migration appears once for another group member with /sync on
- [x] #3 A restart preserves the migrated capability and later edits continue to synchronize
- [x] #4 Fresh ticket-backed channels and point-to-point direct sync behavior remain unchanged
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implementation verification: group members broadcast docs capabilities, deterministically adopt one namespace, persist it, and reannounce it. The regression test starts with divergent legacy documents, adopts one peer index, and receives a subsequent remote entry. Manual testing confirms existing #general now synchronizes.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Migrated legacy group channels to a shared iroh-docs index by exchanging capability announcements over their existing gossip channel. Members deterministically converge on one namespace, persist it across restarts, and preserve fresh ticket-backed and point-to-point sync behavior. Verified by the legacy-index convergence regression, 967 passing tests, and manual #general synchronization.
<!-- SECTION:FINAL_SUMMARY:END -->
