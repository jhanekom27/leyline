---
id: LEY-26.16
title: Migrate existing group channels to shared sync indexes
status: To Do
assignee: []
created_date: '2026-10-14 06:12'
updated_date: '2026-10-14 06:12'
labels:
  - networking
  - files
dependencies:
  - LEY-26.4
modified_files:
  - src/net.rs
  - src/channel_registry.rs
  - src/message.rs
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
- [ ] #1 Two existing members of a ticket-backed group channel that predate docs capabilities converge on one persisted docs namespace without a new invite
- [ ] #2 A stable file synced after migration appears once for another group member with /sync on
- [ ] #3 A restart preserves the migrated capability and later edits continue to synchronize
- [ ] #4 Fresh ticket-backed channels and point-to-point direct sync behavior remain unchanged
<!-- AC:END -->
