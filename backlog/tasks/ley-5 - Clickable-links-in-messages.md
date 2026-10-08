---
id: LEY-5
title: Clickable links in messages
status: To Do
assignee: []
created_date: '2026-10-08 02:11'
labels:
  - messaging
dependencies: []
type: feature
ordinal: 5000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A URL typed into chat currently renders as plain text with no way to open it without leaving the TUI or manually copying the text.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A bare URL in a rendered message is wrapped so supporting terminals make it clickable (for example via OSC 8)
- [ ] #2 Terminals without hyperlink support still show the URL as readable plain text
- [ ] #3 No new external dependency is required
<!-- AC:END -->
