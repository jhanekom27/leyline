---
id: LEY-12
title: 'True direct 1:1 messaging over a point-to-point stream'
status: To Do
assignee: []
created_date: '2026-10-08 02:12'
labels:
  - networking
dependencies: []
type: feature
ordinal: 12000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The existing `/msg` command (Direct 1:1 DMs) still provisions a private gossip channel and requires a one-time ticket exchange. A true point-to-point option, using a direct QUIC stream to a known `EndpointId` instead of gossip, `RoomSecret`, or tickets, would remove that remaining manual step for a peer already met, and would avoid gossip-mesh overhead for a two-node conversation. This is a bigger departure from the one-gossip-task-per-channel model and needs its own connection lifecycle, message framing, presence, and history sync, so it is worth prototyping separately from the rest of the messaging work.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Two peers who already know each other's EndpointId can exchange messages over a direct point-to-point connection, without a ticket exchange
- [ ] #2 The connection recovers from a disconnect via retry/lifecycle handling
- [ ] #3 Message history for the conversation is available after a restart
<!-- AC:END -->
