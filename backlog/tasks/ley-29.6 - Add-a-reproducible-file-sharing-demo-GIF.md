---
id: LEY-29.6
title: Add a reproducible file-sharing demo GIF
status: Done
assignee: []
created_date: '2026-10-15 14:23'
updated_date: '2026-10-20 08:22'
labels:
  - files
  - ux
dependencies: []
parent_task_id: LEY-29
type: docs
ordinal: 75000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Create a separate short VHS recording that demonstrates one peer sharing a fixture with /send and the other retrieving it with /save or /files. Keep the main README connection demo focused while giving file-sharing documentation a concise, reproducible walkthrough.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A committed VHS tape and driver reproducibly render the file-sharing GIF
- [x] #2 The GIF shows one peer using /send and the other using /save or /files to retrieve the same fixture
- [x] #3 The resulting asset is embedded with accessible alt text in the relevant user-facing documentation
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a dedicated file-demo VHS tape and tmux driver under `docs/`, with its own session/socket/state names and GIF output under `guide/src/assets/` so the published guide can serve it directly. The driver will create a small deterministic fixture in Alice’s temporary workspace; use the same isolated-peer, invite-ticket, readiness-wait, cleanup, and release-binary guards as the chat demo.
2. Arrange the recording as Alice, Bob, and a small read-only observer pane for Bob’s per-channel received-files folder. Before sharing, the observer explicitly shows that the deterministic fixture is absent (or that the destination folder has not yet been created); it refreshes after the transfer to list the received fixture. The driver will continue to verify the copy’s expected bytes independently of what the observer displays.
3. After both peers are connected and named, drive Alice to type `/send <fixture-path>`, wait until Bob receives the shared attachment, then drive Bob to type bare `/save`. Wait for the visible saving/success notices before refreshing the observer pane. Keep the visible capture focused on the send/receive flow rather than replaying connection setup.
4. Add a `docs:file-demo` Taskfile target that builds the release binary and invokes the new driver; retain `docs:demo` and the current connection asset unchanged. Extend contributor documentation with the second regeneration command and its distinct output asset.
5. Add `guide/src/file-sharing.md` with a concise manual-sharing walkthrough: the peers must be in the same channel, `/send <path>` shares the fixture, and bare `/save` retrieves the newest peer-shared file into that channel’s folder. Embed the generated GIF with descriptive alt text and register the page in `guide/src/SUMMARY.md`. Add a concise README `File sharing` section after `Why leyline` that embeds the same GIF with accessible alt text, preserving the existing connection/chat GIF as the README hero.
6. Validate with `bash -n` on the new driver, `vhs validate` on the new tape, and `task docs:file-demo` to regenerate and inspect the committed GIF. Then run `task docs:build`, `task docs:test`, and `git diff --check`; confirm the recording visibly shows the fixture absent from Bob’s channel folder before sharing, `/send`, Bob’s received attachment, `/save`, the saved-file confirmation, and the observer listing the fixture after transfer.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented the separate docs/file-demo.sh driver and docs/file-demo.tape with real, isolated Alice and Bob peers. The driver creates Alice's demo-note.txt fixture, waits for Bob to receive it, drives Bob's bare /save, compares Bob's saved bytes, then refreshes a lower observer pane from 'not present yet' to a listing of the saved copy. Added task docs:file-demo, guide/src/file-sharing.md with descriptive GIF alt text, guide navigation, and regeneration guidance. Validation passed: bash -n docs/file-demo.sh; vhs validate docs/file-demo.tape; task docs:file-demo (release build succeeds with the existing unused ChannelRegistry-method warning; real transfer and byte comparison succeed); task docs:build; task docs:test; git diff --check. Visual review confirmed the GIF shows /send, /save, and the received-folder observer before and after transfer.

README now includes the same file-sharing GIF in a concise File sharing section after Why leyline, with accessible alt text and a direct link to the guide. The validated tape and driver are committed in 8645e19 (docs: add file-sharing demo GIF).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a reproducible two-peer file-sharing GIF that shows Alice sharing a deterministic fixture with /send, Bob retrieving it with /save, and Bob's channel folder changing from absent to a listed saved copy. The GIF is embedded with accessible alt text in both the published file-sharing guide and README, while the existing connection/chat GIF remains the README hero. Verified with bash syntax and VHS validation, a real byte-checked task docs:file-demo transfer, task docs:build, task docs:test, and git diff --check.
<!-- SECTION:FINAL_SUMMARY:END -->
