---
id: LEY-26
title: Per-channel shared folders
status: To Do
assignee: []
created_date: '2026-10-12 08:28'
updated_date: '2026-10-12 08:29'
labels:
  - files
  - networking
dependencies: []
type: feature
ordinal: 33000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Moving a file between machines owned by the same person (for example macOS to Linux, where it currently means emailing it to yourself), or sharing a few small files with friends, should not need a command per file. A folder per channel on disk that keeps itself in sync removes that friction.

The work is staged: opt-in auto-save (receive only), a watched outbox (send only), then true two-way sync once a spike has chosen the substrate. Access follows the existing capability model (decision-2): anyone holding the channel ticket is a member, so there is no extra invite step. A private channel shared by your own paired devices (LEY-20) already covers the personal cross-device case and needs no separate feature. The two-way sync subtasks are provisional until the spike lands.
<!-- SECTION:DESCRIPTION:END -->
