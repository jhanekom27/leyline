---
id: LEY-29.3
title: Add a reproducible README terminal demo GIF
status: Done
assignee: []
created_date: '2026-10-15 01:43'
updated_date: '2026-10-15 14:26'
labels: []
dependencies: []
parent_task_id: LEY-29
type: docs
ordinal: 69000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create a short, polished terminal recording that demonstrates two leyline peers chatting live. Use a committed VHS tape/script so contributors can regenerate the GIF, and place the resulting asset prominently in the README.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A committed recording script reproducibly renders the demo GIF
- [x] #2 The GIF shows two peers connecting and exchanging messages in a concise terminal session
- [x] #3 README embeds the GIF near the project introduction with accessible alt text
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add docs/demo.sh as a deterministic tmux driver using isolated temporary HOME directories for Alice and Bob. Capture Bob's real /invite ticket through the clipboard, visibly paste it after Alice types /join, show both peers connecting and exchanging messages, and render docs/assets/leyline-demo.gif from docs/demo.tape. Add a docs:demo Taskfile target and embed the reviewed asset under the README pitch with accessible alt text. Document regeneration in CONTRIBUTING.md.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented docs/demo.sh, docs/demo.tape, docs/assets/leyline-demo.gif, and task docs:demo. The reviewed GIF uses real Alice and Bob instances in tmux with isolated temporary HOME directories; Bob visibly creates #demo and runs /invite, Alice visibly types /join before the captured ticket is pasted, and both exchange messages. Validation: bash -n docs/demo.sh, vhs validate docs/demo.tape, task docs:demo, task docs:build, task docs:test, and git diff --check. The release build reports only existing unused ChannelRegistry methods.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a reproducible, reviewed VHS GIF that shows two real leyline peers creating a channel, sharing an invite, joining, connecting, and chatting. The GIF is embedded below the README pitch and regenerated with task docs:demo.
<!-- SECTION:FINAL_SUMMARY:END -->
