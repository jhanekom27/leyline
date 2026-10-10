---
id: LEY-20.3
title: Keep paired devices in sync after pairing
status: To Do
assignee: []
created_date: '2026-10-10 13:40'
labels:
  - identity
  - channels
dependencies:
  - LEY-20.2
parent_task_id: LEY-20
type: feature
ordinal: 23000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Pairing (see the device-pairing task) only transfers the channel list at pairing time. If a device later joins a new channel, sibling devices belonging to the same person have no way to learn about it short of repeating the whole pairing flow. Add a private channel that every paired device auto-joins, shared only with the other devices of the same person, mirroring the auto-provisioned DM channel pattern already in `dm_registry.rs`, and broadcast a small join event over it whenever a device joins a new channel so sibling devices auto-join too. Also verify and document that file sharing already works across the devices of one person once they share a channel: `/save` already fetches a file by dialing the sender `EndpointId` directly (see `backfill.rs` and `net.rs`), so no new file-transfer code is expected here, only confirmation and, if needed, a regression test.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Every paired device automatically joins a private channel shared only with the other devices of the same person, with no separate invite step
- [ ] #2 Joining a new channel on one device results in sibling devices automatically joining the same channel without user action on the sibling device
- [ ] #3 A file sent with /send from one device can be fetched with /save from a sibling device while the sending device is online
- [ ] #4 Forgetting or un-pairing a device, even a minimal remove-by-id version, stops it from receiving future channel-join broadcasts
<!-- AC:END -->
