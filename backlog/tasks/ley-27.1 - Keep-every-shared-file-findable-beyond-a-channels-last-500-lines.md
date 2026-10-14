---
id: LEY-27.1
title: 'Keep every shared file findable, beyond a channel''s last 500 lines'
status: Done
assignee: []
created_date: '2026-10-13 04:22'
updated_date: '2026-10-14 14:07'
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
- [x] #1 `/files` lists every file shared in the channel, including shares older than its newest 500 transcript lines, newest first, with the same name, size, sender, date and saved marker as for a recent one
- [x] #2 `/save <hash-prefix>` resolves and fetches a file shared outside the window, asking at least its sender and the peers online in its channel
- [x] #3 A bare `/save` still saves the newest file another peer shared in the active channel, exactly as today
- [x] #4 `no files have been shared in this channel yet` appears only for a channel with no file shares at all, however old
- [x] #5 The in-memory transcript cap is unchanged: what is added to find old files holds file metadata only, never old messages
- [x] #6 The same files are found after a restart, and after backfill has delivered older history
- [x] #7 Tests cover a share older than the window being listed, resolved by `/save` and saved, the empty-channel message, and the unchanged transcript cap
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a persistent shared-file metadata index keyed by channel and message ID, preserving distinct shares of the same content. 2. Seed and update it from startup history, live messages, sent files, backfill, and channel leave in main.rs. 3. Refactor /files and /save helpers to query metadata instead of the capped transcript while preserving saved markers and provider behavior. 4. Add old-share, restart, backfill, empty-overlay, and cap-regression tests; run cargo fmt, cargo test, cargo clippy, and cargo build --release.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Verification: durable SharedFiles metadata is seeded from complete startup history, updated for live/sent/backfilled messages, and removed on leave. App tests cover an old share past MAX_SCROLLBACK being listed and resolved by /save, while the transcript remains capped; SharedFiles tests cover reload persistence. Existing overlay, bare-save, empty-channel, and backfill tests remain green. Validation: cargo fmt, cargo test (974 passed), cargo clippy and cargo build --release (only two pre-existing warnings), git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a persistent metadata-only shared-file index so /files and /save can find attachments beyond the 500-line transcript window. The index retains file identity, name, size, sender, and timestamp per share; it is rebuilt from local history at startup and updated for live, sent, and backfilled messages. The transcript cap remains unchanged, while saved markers and provider ordering are preserved.
<!-- SECTION:FINAL_SUMMARY:END -->
