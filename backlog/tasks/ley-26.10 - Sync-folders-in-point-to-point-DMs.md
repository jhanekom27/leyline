---
id: LEY-26.10
title: Sync folders in point-to-point DMs
status: To Do
assignee: []
created_date: '2026-10-13 14:22'
labels:
  - files
  - networking
dependencies:
  - LEY-26.6
parent_task_id: LEY-26
type: feature
ordinal: 56000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend shared-folder sync to /dm point-to-point conversations. Direct peers currently derive distinct local p2p channel names and independently create docs namespaces, so their folder indexes do not converge. Establish and persist one shared docs capability for the direct conversation through the direct protocol without weakening the existing no-ticket direct-message flow.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A /dm conversation has one shared persisted docs namespace on both peers without requiring an extra invite
- [ ] #2 With /sync on on both direct peers, a stable file created in one direct-conversation folder appears once in the other
- [ ] #3 Direct reconnects and normal message delivery remain stable while folder sync is active
- [ ] #4 Existing ticket-backed group-channel sync behavior remains unchanged
<!-- AC:END -->
