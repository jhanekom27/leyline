---
id: LEY-26.9
title: Make the auto-save size limit a device-wide setting
status: To Do
assignee: []
created_date: '2026-10-13 06:56'
labels:
  - files
  - trust-safety
dependencies:
  - LEY-26.1
parent_task_id: LEY-26
type: enhancement
ordinal: 54000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
LEY-26.1 auto-saves incoming files only up to a fixed 25 MB (`files::AUTOSAVE_MAX_FILE_SIZE`). A bigger file from your own device or an aliased peer is skipped, with its `/save` hint, and needs a manual `/save` each time. That is the right default for something that downloads unattended, but the case auto-save exists for -- moving files between your own machines -- is exactly where 25 MB is tight: a video, a disk image, a backup archive. The limit is a compile-time constant with no setting, so today the only way past it for auto-save as a whole is to edit it and rebuild.

It is deliberately not `/maxsize`, which limits what you share (500 MB to begin with) and never what you receive, and not manual `/save`, which has never had a limit; this is only about what is fetched without anyone asking.

Decided while reviewing LEY-26.1: make it a device-wide setting as the starting point, like `/maxsize`. A per-channel limit (large on a channel between your own devices, small on one with friends) would fit the trust model better and is left out here; it can follow if one limit for the whole device turns out to be too blunt. There is always a ceiling someone chose -- a size, never `off` -- because the point of the limit is that a peer cannot fill the disk while nobody is looking.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/autosave limit <size>` sets the largest file auto-save will fetch, for every channel on this device, and `/autosave limit default` restores 25 MB; a size is a number and a unit, read as `/maxsize` reads them, and the choice survives restarts
- [ ] #2 A bare `/autosave`, and `/autosave on` and `/autosave off`, report the limit in force, and the notice for a file skipped as too big names it
- [ ] #3 Something that is not a size, including `off`, is refused with what to type and leaves the limit as it was, so there is always a ceiling
- [ ] #4 The limit is enforced as before whatever its value: against the announced size before any network is touched, and against the real size of the file before it is downloaded, so a sender who understates a file still cannot exceed it
- [ ] #5 Pairing leaves the limit alone, as it does `/maxsize` and the files root: it is a preference of this device
- [ ] #6 Tests cover parsing and refusals, persistence, and both checks against a raised and a lowered limit; README, features.md, concept.md, `/help` and the command hint are updated
<!-- AC:END -->
