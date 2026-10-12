---
id: LEY-25.4
title: Save the newest shared file with a bare /save
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 08:28'
updated_date: '2026-10-12 15:56'
labels:
  - files
  - ux
dependencies: []
parent_task_id: LEY-25
type: enhancement
ordinal: 37000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The `/save` command needs a hash prefix copied out of the transcript, but the common case is simply grabbing the file that was just shared. This mirrors a bare `/reply`, which targets the most recent message. Picking an older file is deliberately left to the `/files` overlay rather than adding a second picker mode.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A bare `/save` saves the most recent attachment from another peer in the active channel, reporting the file name and destination as `/save <hash-prefix>` does today
- [x] #2 When the active channel has no attachment from another peer, a bare `/save` says so instead of doing nothing
- [x] #3 `/save <hash-prefix>` is unchanged
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Resolution (src/app.rs). New `AppState::newest_shared_file`, beside `resolve_attachment` and returning the same `Result<(Hash, String, [u8; 32]), String>`: it walks the active channel transcript (`Channel::messages`) from newest to oldest and returns the first chat message that carries an attachment and was not sent by this device (sender is not `self_id`), else the error `no file from another peer to save in this channel`. That is the same test `ui::attachment_line` uses to give a file its `/save <hash> to download` hint, so a bare `/save` takes exactly the newest file that shows one (a file from one of our own paired devices included: it is not on this device). It reads the channel transcript rather than `visible_chat_ids`: an open thread or search narrows what `/reply` can target, but the file that was just shared belongs to the channel, not to the view. System lines are skipped.
2. `run_save` picks its file with `newest_shared_file` when no argument is given and with `resolve_attachment` otherwise; both then share the existing tail: the `saving <file>...` breadcrumb and `InputAction::SaveFile`. So the download, the destination report (`saved <file> to <path>`) and the failure notices are the ones `/save <hash-prefix>` has today, and the prefix path is untouched. No change to main.rs, net.rs or files.rs; the `/save` usage notice goes away.
3. Docs: `/save [hash-prefix]` in the /help text and the sidebar hint (`COMMAND_HINTS`), a `/save` row beside `/save <hash-prefix>` in the README Commands table (as `/send` and `/reply` have), a features.md entry under Files and rich content, and the `InputAction::SaveFile` doc comment. concept.md needs nothing (no new module).
4. Tests (app.rs): the newest of several wins; our own files are skipped even when newer; text messages and system lines newer than the file do not hide it; an open search does not narrow it; only the active channel is read; a paired sibling device counts as another peer; no file at all, and only our own files, each say so and return no action; `/save <hash-prefix>` still reaches an older file. The obsolete test that pinned the bare `/save` usage notice is replaced: AC 1 replaces that behavior.
5. Verify with cargo test, cargo clippy --all-targets and cargo fmt --check, then the real binary: two instances under isolated HOMEs, one shares files and the other types a bare `/save` (newest file saved into Downloads/leyline and reported; a bare `/save` in a channel with nothing from a peer says so; `/save <prefix>` still saves an older file).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Built as planned, in src/app.rs only: AppState::newest_shared_file returns the newest chat line of the active channel transcript that carries an attachment and was not sent by this device, else the error "no file from another peer to save in this channel"; run_save picks its file with it when there is no argument and with resolve_attachment otherwise, and both then share the one tail (the "saving <file>..." line and InputAction::SaveFile). main.rs, net.rs and files.rs are untouched, so the download, the "saved <file> to <path>" report and the failure notices are exactly those of /save <hash-prefix>. Also updated: /save [hash-prefix] in /help and the sidebar hint (COMMAND_HINTS), a /save row in the README Commands table, and a features.md entry. The old test that pinned a bare /save printing a usage notice is replaced: AC 1 replaces that behavior.

Validation: cargo test 725 passed (10 new in app.rs, one obsolete test removed, was 716), cargo clippy --all-targets clean, and every file touched is cargo fmt clean (cargo fmt --check still flags an old hunk in src/dm_registry.rs that this task does not touch; left alone). The real binary, two instances under isolated HOMEs each in a PTY, screens read with pyte, B joined to the #general of A with --join and sharing files: 12 of 12 checks pass. AC 1: after B shared first.txt then second.txt, a bare /save on A printed "saving second.txt..." then "saved second.txt to <HOME>/Downloads/leyline/second.txt", the file held the bytes of second.txt and first.txt was not saved; with a newer mine.txt of A in the channel, a bare /save still saved second.txt again (as second (1).txt) and never mine.txt. AC 2: in a channel with nothing shared a bare /save said "no file from another peer to save in this channel" and saved nothing; the same in a second channel (#other) while #general held files, and in #other holding only the own file of A. AC 3: /save with the hash prefix of first.txt still saved the older first.txt, and /save not-hex and /save ffffffff still report "invalid hash prefix" and "no known file matches". Also checked: an open /search narrowed to first.txt did not narrow a bare /save, which still saved second.txt; the sidebar shows /save [hash] and /help lists /save [hash-prefix].

Interpretations to confirm: (1) Another peer means anyone but this device (sender is not self_id), the test ui::attachment_line already uses to give a file its /save hint, so a bare /save takes exactly the newest file that shows one; a file from one of your own paired devices therefore counts (it is not on this one), even though the transcript labels that sender "you". (2) Most recent is by position in the loaded transcript of the active channel, like a bare /reply: the newest 500 lines kept in memory per channel, history loaded at startup included; a file older than that is out of reach of a bare /save, as it is of /save <hash-prefix>. (3) It reads the channel transcript, not the view: an open thread or search narrows what /reply can target but does not narrow a bare /save. (4) A second bare /save with nothing newer in between saves the same file again, as name (1).ext, as running /save <hash-prefix> twice does. (5) One notice covers both no file at all and only your own files.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
A bare /save now saves the newest file another peer shared in the active channel instead of printing a usage notice, so the common case needs no hash copied out of the transcript. AppState::newest_shared_file walks the channel transcript newest first for a chat line with an attachment whose sender is not this device (the test that already gives a file its /save hint), and run_save sends that file, or the one a hash prefix names, through the same "saving <file>..." line and InputAction::SaveFile; downloads, the "saved <file> to <path>" report and failures are therefore exactly those of /save <hash-prefix>, which is unchanged. With no such file it says "no file from another peer to save in this channel". /help, the sidebar hint, the README and features.md are updated. Verified with 725 passing tests (10 new) and clean clippy, and 12 scripted checks of two real instances under PTYs covering every acceptance criterion: the newest of two shared files saved with its name and destination reported, a newer file of our own skipped, an empty channel and an own-files-only channel saying so (also a second channel while the first held files), a prefix still saving an older file, and an open search not narrowing it.
<!-- SECTION:FINAL_SUMMARY:END -->
