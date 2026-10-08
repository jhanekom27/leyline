---
id: LEY-11
title: Document channel-secret rotation as the remove-a-member story
status: To Do
assignee: []
created_date: '2026-10-08 02:12'
labels:
  - trust-safety
dependencies: []
type: feature
ordinal: 11000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
leyline's capability/gossip model means whoever holds the `RoomSecret` can always rejoin, so there is no built-in ban mechanism. The only real way to remove a compromised or unwanted member today is to rotate to a fresh `RoomSecret` and re-invite everyone else, but this is not written down anywhere, making it easy to later be tempted to bolt on a ban-list that gossip cannot actually enforce.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 README or concept.md explicitly documents secret rotation as the recommended way to remove a member from a channel
- [ ] #2 The documentation explains why a gossip-based ban list would not work under the current trust model
<!-- AC:END -->
