---
id: LEY-27.1
title: 'Keep every shared file findable, beyond a channel''s last 500 lines'
status: To Do
assignee: []
created_date: '2026-10-13 04:22'
labels:
  - files
  - ux
dependencies: []
parent_task_id: LEY-27
priority: low
type: enhancement
ordinal: 53000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Each channel keeps only its newest 500 transcript lines in memory (`MAX_SCROLLBACK` in `src/app.rs`). History loaded at startup and history backfilled from peers are trimmed to that window, and local notices such as `saving...` and `saved...` count against it too. Everything that looks a shared file up reads the same window: the `/files` overlay (LEY-25.7), `/save <hash-prefix>` and a bare `/save` (`known_attachments`, `carries_file`, `channel_of_file` and `providers_for` in `src/app.rs`).

So a file shared further back than that is simply not there. `/files` does not list it, `/save <hash-prefix>` answers `no known file matches`, and a channel whose files are all older shows `no files have been shared in this channel yet`, which is not true. `/search` does scan the full on-disk log and can show such a file's message, but the `/save <hash>` hint on it fails the same way. Nothing is actually lost -- the message is in the channel's on-disk log, the file is in the blob store (never garbage-collected), and any saved copy is in its channel folder -- it is only out of reach from the UI, and the old files are the ones a file browser is most wanted for.

The 500-line cap bounds render and memory cost in a long-lived session (concept.md) and should stay as it is. What is missing is a way to look up file shares that does not depend on it. Interpretation (1) in LEY-25.7's notes is where this limit was first recorded.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `/files` lists every file shared in the channel, including shares older than its newest 500 transcript lines, newest first, with the same name, size, sender, date and saved marker as for a recent one
- [ ] #2 `/save <hash-prefix>` resolves and fetches a file shared outside the window, asking at least its sender and the peers online in its channel
- [ ] #3 A bare `/save` still saves the newest file another peer shared in the active channel, exactly as today
- [ ] #4 `no files have been shared in this channel yet` appears only for a channel with no file shares at all, however old
- [ ] #5 The in-memory transcript cap is unchanged: what is added to find old files holds file metadata only, never old messages
- [ ] #6 The same files are found after a restart, and after backfill has delivered older history
- [ ] #7 Tests cover a share older than the window being listed, resolved by `/save` and saved, the empty-channel message, and the unchanged transcript cap
<!-- AC:END -->
