---
id: LEY-25.8
title: Allow overriding the file size limit when sending deliberately
status: To Do
assignee: []
created_date: '2026-10-13 02:53'
labels:
  - files
  - ux
dependencies:
  - LEY-25.1
parent_task_id: LEY-25
type: enhancement
ordinal: 51000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
`files::MAX_FILE_SIZE` (500 MB, `src/files.rs:48`) hard-refuses any file or pasted image over the cap in `stage_file`/`stage_image`, with no way around it short of editing the constant and rebuilding. It is a good guardrail against an accidental huge share, but it blocks a deliberate large send (a video, a backup archive) just as hard as it blocks a mistake.

Keep the default guardrail intact -- normal-sized sends should see no change -- but add a way past it for a deliberate send: a persisted setting that raises or disables the cap, a one-off confirmation for that specific file, or both. Which mechanism(s) to build is an implementation-time decision.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 With no override, staging or sending a file over the limit is refused exactly as today -- the default guardrail is unchanged
- [ ] #2 A user who deliberately wants to share an over-limit file has a way to do so without rebuilding the binary
- [ ] #3 The refusal message for an oversized file says how to override it
- [ ] #4 Any persisted override survives restarts and is discoverable, the way /folder shows the files root today
- [ ] #5 Tests cover the default refusal and the override path
<!-- AC:END -->
