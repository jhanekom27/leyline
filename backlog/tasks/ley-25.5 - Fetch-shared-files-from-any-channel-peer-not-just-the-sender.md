---
id: LEY-25.5
title: 'Fetch shared files from any channel peer, not just the sender'
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 08:28'
updated_date: '2026-10-13 00:03'
labels:
  - files
  - networking
dependencies: []
parent_task_id: LEY-25
type: enhancement
ordinal: 38000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The `/save` command only contacts the peer who originally shared a file, so the file cannot be fetched whenever that peer is offline, even though anyone who already downloaded it serves it from their own blob store. Offering every known peer of the channel as a provider lets files outlive the session of their sender. No wire change is needed.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 `/save` offers the original sender and the other peers known in the channel as providers, and succeeds when the sender is offline but another peer already holds the file
- [x] #2 A peer that does not have the file never makes the save fail while another provider can serve it
- [x] #3 When no reachable peer has the file, the failure message says so (and that the sender may be offline) instead of showing a raw network error
- [x] #4 A test covers a file held only by a peer other than the sender
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Where a peer serves a file from (finding, no code needed). iroh-blobs serves from the peer's own blob store, never from the Downloads copy: `/save` downloads into the store (the downloader in `BackfillStore::fetch_file`) and only then `export`s a copy (`Blobs::export` is `ExportMode::Copy` in iroh-blobs 0.103; only `TryReference` would move data out of the store), and `/send` imports with `ImportMode::Copy`. So a peer that saved a file can keep serving it after moving or deleting the exported copy, and the sender after deleting the original. The store is opened with `FsStore::load` (`Options::new` has `gc: None`), so nothing is ever collected. No wire change and no check that the exported file is still there.
2. Provider list (src/app.rs). New `AppState::providers_for(hash, sender)` returns who to ask, best bet first: the sender; then the peers currently online in the channels that carry this file (`Channel::peers`); then everyone who has posted in those channels (gossip only keeps a handful of direct neighbors, so a holder may not be one of them). This device is left out unless it is the sender (a file we sent is already in our store), and each id appears once. "Channels that carry the file" = every joined channel whose transcript has a message with this hash, so `/save <prefix>` for a file from another channel and a bare `/save` agree. `run_save` calls it and `InputAction::SaveFile` carries `providers: Vec<[u8; 32]>` in place of `sender`; main.rs and `Net::save_file` pass it through.
3. Fetching (src/backfill.rs). `fetch_file` takes `providers: &[[u8; 32]]`. It first looks in our own store: a blob we already hold (a file we sent, or saved before) is exported without asking anyone. That check is explicit because the downloader only looks at local state inside its provider loop, so with no provider listed it fails even for a blob we hold. Otherwise it hands every provider to the downloader, which tries them one at a time in the order given and moves on when one cannot connect or cannot serve the blob (AC 2). Ids come from peers' messages, so one that is not a valid key is skipped instead of failing the save for the others. When no provider serves it the error says `no reachable peer has it right now -- its sender may be offline` (AC 3) and the downloader's own error goes to the log at debug. An offline provider costs at most the downloader's 1s connect timeout.
4. Tests. backfill.rs, with real endpoints over loopback and a small `TestPeer` helper: A shares, B saves and then deletes its Downloads copy, A goes offline, C saves from [A, B] (AC 1 and 4, and the moved-file question); an online peer without the file listed first does not fail the save (AC 2); every provider failing gives the friendly message and not the raw error (AC 3); a blob already in the local store saves with every provider unreachable or none listed; an invalid provider id is skipped. app.rs: provider order and contents, no self, each id once, only channels that carried the file; the existing `/save` tests adapt to `providers`.
5. Docs: the `/save` rows in the README, a features.md entry, a short concept.md section on where files are served from, and the comments that describe `/save` as sender-only (the backfill regression test, `resolve_attachment`, `InputAction::SaveFile`, `Net::save_file`).
6. Verify with cargo test, cargo clippy --all-targets and cargo fmt --check, then the real binary: three instances under isolated HOMEs, A shares two files, B saves one and its Downloads copy is removed, A quits, C saves it; and C tries the file nobody else holds.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Built as planned, with one change: `BackfillStore::fetch_file` now looks in our own blob store before it involves the downloader. Mutating the code showed the downloader only checks local state inside its provider loop, so with an empty provider list it fails even for a blob we hold; the explicit check removes that dependence (a file we sent or saved before is exported without asking anyone, whatever the list).

Changed: src/app.rs (`InputAction::SaveFile` carries `providers: Vec<[u8; 32]>` in place of `sender`; new `AppState::providers_for` and `Channel::carries_file`; `run_save` uses them), src/net.rs and src/main.rs (pass the list through), src/backfill.rs (`fetch_file`), README (the two `/save` rows), features.md (new entry), concept.md (new File sharing section).

The moved-file question: a peer serves from its own iroh-blobs store, not from the Downloads copy. `/save` downloads into the store and then exports a copy (`ExportMode::Copy`), `/send` imports a copy (`ImportMode::Copy`), and `FsStore::load` configures no garbage collection. Read in the iroh-blobs 0.103 source and checked by a test: A shares, B saves and deletes its Downloads copy, A goes offline, C saves from B. Exporting by `ExportMode::TryReference` instead (the store then moves its data out to the exported path and references it) makes that test fail, so it does detect the problem.

Validation: cargo test 736 passed (11 new: 5 in backfill.rs, 6 in app.rs; was 725), cargo clippy --all-targets clean, and cargo fmt --check flags only the old hunk in src/dm_registry.rs that this task does not touch (left alone). Mutation checks: sender-only fetching (`take(1)`) fails the three tests that need a second provider, and the TryReference export fails the moved-file test. The real binary, three instances under isolated HOMEs each in a PTY read with pyte (scratch script outside the repo): 16 of 16 checks pass. A, B and C each see two peers; A shares unsaved.bin then report.bin; B's bare /save saves report.bin (byte-identical) and B's copy in Downloads is removed; A quits; C's bare /save then saves report.bin byte-identical to what A shared, with A offline and B's Downloads copy gone, so it came from B's store; C's /save of unsaved.bin, held by nobody still online, prints "failed to save unsaved.bin: no reachable peer has it right now -- its sender may be offline", never shows the downloader's "Unable to download", leaves no file in Downloads, and the cause ("no provider served the file") is in C's leyline.log.

Interpretations to confirm: (1) "Other peers known in the channel" = the peers online in the channels that carry the file (gossip neighbors), then everyone who has posted there (the loaded transcript), after the sender. Someone who has the file, is online, is not one of the few direct gossip neighbors and has never posted is not known to us and is not asked. (2) A file shared in several channels asks all of them, so the list does not depend on which channel /save resolved it in. (3) The list is not capped: an unreachable provider costs up to the downloader's 1s connect timeout, so a save nobody can serve, in a channel with many past posters, takes about that many seconds to say so; a cap is a small change if wanted. (4) Any download failure is reported as "no reachable peer has it": the downloader counts a local write failure as a provider failure too, so they cannot be told apart; the raw cause is in the log at debug. (5) A peer serving what it saved depends on the store never being garbage-collected (`FsStore::load` configures none); if collection is ever turned on, downloaded blobs carry no tag and would be collected, so a tag on download would be needed.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
`/save` no longer depends on the sender being online. `AppState::providers_for` offers the sender first, then the peers online in the channels that carry the file, then everyone who has posted there (never this device, except as the sender), and `BackfillStore::fetch_file` hands them all to the iroh-blobs downloader, which tries them one at a time and moves on from one that is offline or lacks the blob. Peers serve from their own blob store, not from their Downloads folder (`/save` exports a copy, `/send` imports one), so a peer that saved a file keeps serving it after moving or deleting its copy. When nobody can serve it the failure says "no reachable peer has it right now -- its sender may be offline" instead of the downloader's raw error; a blob already in our own store is exported without asking anyone, and a provider id that is not a valid key is skipped. Verified with 736 passing tests (11 new, real endpoints over loopback, mutation-checked against sender-only fetching and a TryReference export), clean clippy, and 16 scripted checks of three real instances under PTYs: A shared two files, B saved one and deleted its Downloads copy, A quit, C saved it from B and got the failure message for the file nobody else held. README, features.md and concept.md are updated.
<!-- SECTION:FINAL_SUMMARY:END -->
