---
id: LEY-24.2
title: Replace the top tab bar with a left-side Channels and People navigation panel
status: To Do
assignee: []
created_date: '2026-10-12 05:45'
labels:
  - ux
  - channels
dependencies:
  - LEY-24.1
parent_task_id: LEY-24
type: feature
ordinal: 29000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The tab bar (`ui::render_header`) is a single horizontal row with no overflow or scroll handling -- once there are more channels and DMs or point-to-point chats than fit the terminal width, tabs become unreadable, and there is no visual grouping between ordinary group channels and 1:1 conversations beyond the name prefix. This replaces it with a scrollable left-side panel, split into a group-channels section and a people section (using the kind classification from the channel-kind task), with each row showing a kind glyph and the resolved display name. `Tab` and `Shift+Tab` keep moving through the combined list exactly as they do today, with no new modal focus state, so this is a rendering and layout change rather than a change to the interaction model.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Channels are listed in a left-side panel grouped into two sections -- group channels, and DMs/point-to-point chats (for example labeled CHANNELS and PEOPLE) -- instead of the current single-row top tab bar
- [ ] #2 Each entry in the people section shows a glyph distinguishing a gossip-based DM from a point-to-point chat
- [ ] #3 The panel scrolls when the combined list is taller than the available space, instead of truncating or overflowing
- [ ] #4 `Tab` and `Shift+Tab` continue to switch the active channel, in the same relative order as before
- [ ] #5 The active channel is visually highlighted in its row, mirroring the current active-tab styling
<!-- AC:END -->
