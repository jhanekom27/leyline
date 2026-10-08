---
id: LEY-7
title: Persist last-seen timestamps for peers
status: To Do
assignee: []
created_date: '2026-10-08 02:11'
labels:
  - channels
dependencies: []
type: feature
ordinal: 7000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Presence is purely ephemeral today, driven only by live `NeighborUp`/`NeighborDown` gossip events (see concept.md's Presence section). A peer who goes offline simply disappears from the sidebar instead of being remembered.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 The last time a peer was seen online is persisted per peer, per channel
- [ ] #2 A peer who is not currently connected but has a recorded last-seen time is still listed, visually distinguished from online peers, instead of disappearing entirely
<!-- AC:END -->
