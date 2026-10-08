---
id: LEY-10
title: Mute or block a peer
status: To Do
assignee: []
created_date: '2026-10-08 02:11'
labels:
  - trust-safety
dependencies: []
type: feature
ordinal: 10000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
There is currently no client-side way to stop seeing messages from a specific, unwanted peer within a channel.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A user can mute or block a specific peer by endpoint id
- [ ] #2 Messages from a muted or blocked peer are hidden from the transcript for the user who muted them
- [ ] #3 The mute/block list persists across restarts, stored alongside contacts
- [ ] #4 No network-visible change occurs for the blocked peer; the block is purely client-side
<!-- AC:END -->
