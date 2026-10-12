---
id: LEY-25.5
title: 'Fetch shared files from any channel peer, not just the sender'
status: To Do
assignee: []
created_date: '2026-10-12 08:28'
labels:
  - files
  - networking
dependencies: []
parent_task_id: LEY-25
type: enhancement
ordinal: 38000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The `/save` command only contacts the peer who originally shared a file, so the file cannot be fetched whenever that peer is offline, even though anyone who already downloaded it serves it from their own blob store. Offering every known peer of the channel as a provider lets files outlive the session of their sender. No wire change is needed.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/save` offers the original sender and the other peers known in the channel as providers, and succeeds when the sender is offline but another peer already holds the file
- [ ] #2 A peer that does not have the file never makes the save fail while another provider can serve it
- [ ] #3 When no reachable peer has the file, the failure message says so (and that the sender may be offline) instead of showing a raw network error
- [ ] #4 A test covers a file held only by a peer other than the sender
<!-- AC:END -->
