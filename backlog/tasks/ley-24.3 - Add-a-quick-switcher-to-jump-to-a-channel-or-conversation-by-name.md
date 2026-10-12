---
id: LEY-24.3
title: Add a quick-switcher to jump to a channel or conversation by name
status: To Do
assignee: []
created_date: '2026-10-12 05:45'
labels:
  - ux
  - channels
dependencies:
  - LEY-24.2
parent_task_id: LEY-24
type: feature
ordinal: 30000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Once the channel and people list is long (see the navigation panel task), stepping through it one at a time with `Tab` and `Shift+Tab` does not scale. This adds a filterable jump-to-conversation overlay, mirroring the existing pick-mode pattern already used for `Ctrl+R` (reply target) and `Ctrl+T` (thread view): invoke it, type to filter the combined channel and people list by name, move the highlight, and confirm to switch.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A dedicated keybinding opens an overlay listing every channel and conversation, filterable by typing
- [ ] #2 Typing narrows the list to entries whose resolved name contains the typed text, case-insensitive
- [ ] #3 Confirming a highlighted entry switches to it and closes the overlay; cancelling leaves the active channel unchanged
<!-- AC:END -->
