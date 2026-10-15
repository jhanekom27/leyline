---
id: LEY-29.4
title: Rewrite README introduction as a project pitch
status: Done
assignee: []
created_date: '2026-10-15 01:43'
updated_date: '2026-10-15 14:26'
labels: []
dependencies: []
parent_task_id: LEY-29
type: docs
ordinal: 70000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Restructure the README opening around leyline's user value: private peer-to-peer terminal chat with no servers or accounts. Move contributor-oriented status, roadmap, and Backlog workflow details into CONTRIBUTING.md or focused documentation.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 README opens with a concise user-facing value proposition and a small feature summary
- [x] #2 The demo media appears before detailed build and architecture material
- [x] #3 Contributor-specific workflow material is moved out of the user-facing introduction and remains discoverable
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Rewrite the README opening as a concise user pitch: private peer-to-peer terminal chat, no accounts or central server, private invite-based channels, offline history, and optional shared folders. Move the technical status/roadmap and Backlog workflow material into CONTRIBUTING.md while keeping build/run instructions user-focused. LEY-29.3 owns the missing terminal GIF and committed VHS source; once its asset exists, place it directly under the README pitch before build/architecture details. Validate Markdown structure and the guide build; leave this task In Progress until that media dependency lands.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Rewrote the README opening as a concise user-facing pitch with a small feature summary, and moved roadmap/task-management material into CONTRIBUTING.md. Removed premature contributor instructions for a nonexistent demo source. Validation: git diff --check and task docs:build/test passed. Acceptance criterion 2 remains pending because LEY-29.3 owns the missing reproducible terminal GIF and committed recording source; no placeholder media was added.

LEY-29.3 supplied the approved reproducible terminal GIF. It is embedded directly beneath the README project pitch with accessible alt text, before build and architecture material. Final validation: task docs:demo, task docs:build/test, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Rewrote the README as a user-facing pitch with a concise feature summary, moved roadmap and task-management material into CONTRIBUTING.md, and placed the reviewed two-peer terminal demo directly below the introduction. Verified through the reproducible VHS demo and documentation build/test.
<!-- SECTION:FINAL_SUMMARY:END -->
