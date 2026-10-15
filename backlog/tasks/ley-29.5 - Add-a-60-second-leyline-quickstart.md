---
id: LEY-29.5
title: Add a 60-second leyline quickstart
status: Done
assignee: []
created_date: '2026-10-15 01:43'
updated_date: '2026-10-15 14:36'
labels: []
dependencies: []
parent_task_id: LEY-29
type: docs
ordinal: 72000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Document the shortest path for two people to install leyline, exchange an invite ticket, join the same private channel, and send a first message.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Quickstart covers install, first run, /invite, and /join in a short sequential flow
- [x] #2 Instructions work for two separate users or clearly describe local multi-instance testing
- [x] #3 Quickstart is easy to find from the README
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add a concise guide quickstart and README entry point using the current development build path. It will show Alice starting leyline, creating #demo with /join demo, inviting Bob with /invite, and Bob starting separately then using /join <ticket>; use separate HOME directories for local testing without relying on the conflicting fresh #general channel. Create a follow-up docs task under LEY-29 that depends on LEY-30, LEY-31, and LEY-32 to refresh installation and launch guidance after those workstreams land. Validate with mdBook build/test and git diff --check.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added guide/src/quickstart.md and linked it from guide navigation plus the README. The current quickstart uses git clone/cargo build, starts two isolated local peers, creates #demo with /join demo, generates /invite, and has the second peer /join the ticket before chatting. The README build section now points to this technically accurate flow instead of suggesting a fresh #general ticket join. Validation: task docs:build, task docs:test, and git diff --check passed. LEY-29.7 tracks the planned installation/launch refresh after LEY-30, LEY-31, and LEY-32.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a discoverable 60-second quickstart for building leyline, creating a private #demo channel, inviting a second peer, joining with the ticket, and sending a first message. It supports both real people and isolated local testing, and has a dependency-tracked refresh task for future installation paths.
<!-- SECTION:FINAL_SUMMARY:END -->
