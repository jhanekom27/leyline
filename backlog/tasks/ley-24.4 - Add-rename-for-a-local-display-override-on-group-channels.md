---
id: LEY-24.4
title: Add /rename for a local display override on group channels
status: To Do
assignee: []
created_date: '2026-10-12 05:46'
labels:
  - ux
  - channels
dependencies:
  - LEY-24.1
parent_task_id: LEY-24
type: feature
ordinal: 31000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Group channels only ever show the name they were created or joined under, with no way to shorten or relabel one locally when it is inconvenient, for example a long project name, or two similarly named channels from different friends. `/alias` already covers this for people -- renaming here is specific to group channels, since a DM or point-to-point conversation displayed name already resolves from the peer alias (see the channel-kind task), and having two independent overrides for the same 1:1 conversation would be confusing. This is a local-only override that is never sent over the wire, mirroring the pet-name pattern in contacts.rs.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/rename <name>` sets a local display override for the active group channel, shown in the navigation panel and anywhere else the channel name is displayed
- [ ] #2 The override persists across restarts
- [ ] #3 Running `/rename` on a DM or point-to-point conversation is refused with a message pointing to `/alias` instead
- [ ] #4 Leaving a renamed channel forgets its override, so a later rejoin under the same name does not inherit a stale display name
<!-- AC:END -->
