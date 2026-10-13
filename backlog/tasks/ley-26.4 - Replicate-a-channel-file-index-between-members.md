---
id: LEY-26.4
title: Replicate a channel file index between members
status: To Do
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 08:27'
labels:
  - files
  - networking
dependencies:
  - LEY-26.3
parent_task_id: LEY-26
type: feature
ordinal: 44000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provisional until the spike lands. Every member of a channel needs to converge on the same view of which files exist in its folder (path, content hash, size, modified time, author) with no server, including peers that were offline. This task replicates that index only and does not touch the disk.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Each channel has a replicated file index that only holders of the channel ticket can read or write, with no extra invite step
- [ ] #2 An entry added on one instance appears on the other members of the channel without user action, and a peer that was offline converges when it reconnects
- [ ] #3 Index state persists across restarts and is removed when the channel is left
- [ ] #4 Existing channels, DMs, and history backfill behave exactly as before
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LEY-26.3 substrate decision: implement the replicated index as one persisted iroh-docs namespace per channel. Store path-keyed entries with content hash, size, modified time, and author metadata; use per-device docs authors and Query::single_latest_per_key for the resolved index. Persist the docs namespace/write capability with channel state, distribute it only via channel ticket/pairing flows, and route docs live events into the runtime event loop.
<!-- SECTION:NOTES:END -->
