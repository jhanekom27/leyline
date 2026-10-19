---
id: LEY-39
title: Present /who peers in a details overlay
status: To Do
assignee: []
created_date: '2026-10-19 08:35'
labels:
  - ux
  - commands
dependencies: []
priority: medium
type: enhancement
ordinal: 82000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Replace the transcript-style /who output with a compact, dismissible peer-details overlay for the active channel.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Running /who opens an overlay that lists each active-channel peer in a table with endpoint ID, local alias, and broadcast nickname.
- [ ] #2 Missing aliases or nicknames and long endpoint IDs remain readable without breaking the overlay layout.
- [ ] #3 Escape dismisses the overlay and returns to the prior conversation state.
<!-- AC:END -->
