---
id: LEY-26.1
title: Opt-in auto-save of incoming files per channel
status: To Do
assignee: []
created_date: '2026-10-12 08:29'
labels:
  - files
  - trust-safety
dependencies:
  - LEY-25.6
parent_task_id: LEY-26
type: feature
ordinal: 41000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Receiving never downloads anything automatically, on purpose: any channel member could otherwise fill your disk. For channels you trust, such as a private channel between your own devices, the manual `/save` step is pure friction, so make auto-save an explicit per-channel opt-in with guardrails.

Open question: whether to accept files only from peers you have given a local alias (`/alias`), or from any member. LEY-10 (mute or block a peer) and LEY-9 (fingerprint verification) should feed into this once they exist.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/autosave on` and `/autosave off` switch auto-save for the active channel; a bare `/autosave` reports its state and size limit; it is off by default and the setting survives restarts
- [ ] #2 With auto-save on, a file shared by another peer after it was enabled is downloaded into the channel folder without any command when it is within the size limit; a larger file is left with the usual `/save` hint and a notice saying why
- [ ] #3 Files that arrive through history backfill, or were shared before auto-save was enabled, are never downloaded automatically
- [ ] #4 Auto-saved files use the same filename sanitization and collision handling as `/save`, and nothing is ever opened automatically
- [ ] #5 The transcript shows where an auto-saved file went, or why one was skipped or failed
<!-- AC:END -->
