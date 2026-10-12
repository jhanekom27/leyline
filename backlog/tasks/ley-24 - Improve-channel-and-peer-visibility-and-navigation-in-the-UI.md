---
id: LEY-24
title: Improve channel and peer visibility and navigation in the UI
status: To Do
assignee: []
created_date: '2026-10-12 05:45'
labels:
  - ux
  - channels
dependencies: []
type: feature
ordinal: 27000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DM and point-to-point channel tabs are named from the hex-prefix of a peer at creation time and never update, so a channel can stay `dm-a1b2c3d4` or `p2p-a1b2c3d4` forever even after that peer is later `/alias`ed -- the same hex-id problem features.md already called out and fixed for in-channel peer references (petnames, nicknames via `display_name`), but never threaded through to channel or tab labels. There is also no visual distinction between a group channel, a gossip-based DM, and a point-to-point chat beyond reading the `dm-`/`p2p-` name prefix, and the single-row top tab bar has no overflow or scroll handling and no quick-jump for navigating many channels at once. This parent tracks a structural pass on channel and peer visibility and navigation: classifying channel kind, replacing the top tab bar with a scrollable left-side navigation panel split into Channels and People sections, a quick-switcher for jumping to a channel by name, and a manual rename escape hatch for group channels.
<!-- SECTION:DESCRIPTION:END -->
