---
id: LEY-24.1
title: Classify channel kind and resolve DM and point-to-point tab names live
status: To Do
assignee: []
created_date: '2026-10-12 05:45'
labels:
  - ux
  - channels
dependencies: []
parent_task_id: LEY-24
type: feature
ordinal: 28000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
`/msg` and `/dm` name a DM or point-to-point channel from the pet name or hex-prefix of a peer at the moment the channel is created (`AppState::unique_dm_channel_name`, `direct::channel_name`), but nothing re-evaluates that name afterward. Running `/alias` on a peer updates `AppState::display_name` everywhere it is already used (the peers sidebar, `/who`), but not the tab label of the channel itself, so a DM or point-to-point chat started before a peer name was known can stay a bare hex-prefix forever. Fixing the display is only half of it: today the only way to tell whether a given channel is an ordinary group channel, a gossip-based DM (`dm_channels`), or a point-to-point chat (`direct_channels`) is to scan those two maps by hand or parse the `dm-`/`p2p-` name prefix -- there is no single place that answers what kind a channel is and who it is with. This is the foundation the UI work in this epic (the nav panel, kind badges, quick-switcher) builds on.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 The displayed name of a DM or point-to-point channel reflects the current pet name or nickname resolution for its peer (`AppState::display_name`), rather than a name frozen at channel-creation time, including updating immediately after running `/alias` on that peer
- [ ] #2 A single `AppState` query classifies any channel by kind (group, gossip DM, or point-to-point) and, for the latter two, returns the associated peer id
- [ ] #3 Group channel names are unaffected by this change and continue to show their stored name rather than a resolved one
<!-- AC:END -->
