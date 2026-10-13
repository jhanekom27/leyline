---
id: LEY-26.1
title: Opt-in auto-save of incoming files per channel
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 07:06'
labels:
  - files
  - trust-safety
dependencies:
  - LEY-25.6
parent_task_id: LEY-26
type: feature
ordinal: 41000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Receiving never downloads anything automatically, on purpose: any channel member could otherwise fill your disk. For channels you trust, such as a private channel between your own devices, the manual `/save` step is pure friction, so make auto-save an explicit per-channel opt-in with guardrails.

Open question: whether to accept files only from peers you have given a local alias (`/alias`), or from any member. LEY-10 (mute or block a peer) and LEY-9 (fingerprint verification) should feed into this once they exist.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 `/autosave on` and `/autosave off` switch auto-save for the active channel; a bare `/autosave` reports its state and size limit; it is off by default and the setting survives restarts
- [x] #2 With auto-save on, a file shared by another peer after it was enabled is downloaded into the channel folder without any command when it is within the size limit; a larger file is left with the usual `/save` hint and a notice saying why
- [x] #3 Files that arrive through history backfill, or were shared before auto-save was enabled, are never downloaded automatically
- [x] #4 Auto-saved files use the same filename sanitization and collision handling as `/save`, and nothing is ever opened automatically
- [x] #5 The transcript shows where an auto-saved file went, or why one was skipped or failed
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Findings (no code needed).
- The save pipeline already does everything after the decision: `AppState::save_request` -> `InputAction::SaveFile` -> `files::resolve_destination` (sanitizing, `name (1).ext`) -> `Net::save_file` -> `BackfillStore::fetch_file` -> `NetEvent::FileSaved`, into `files::channel_dir` (LEY-25.6). Auto-save is a second way to reach that pipeline, not a second pipeline.
- Live and everything else are already separate paths. Only `NetEvent::Received` comes back from `handle_net_event` as a newly accepted message (the `other =>` arm in main.rs); backfill goes through `HistoryFetched`/`merge_history` and startup history through `load_history`. Hooking only the live arm gives AC 3 by construction, with nothing retroactive when auto-save is turned on. "After it was enabled" is judged by arrival, never by `ts_unix_ms`, which the sender writes.
- `ChatMessage.sender` is self-asserted. Neither `net::forward_events` (gossip) nor `direct::receive_loop` (`/dm`) checks it against anything, and no chat message is signed (only `DeviceCert` is). Any channel member can claim to be your other device or an aliased friend, so a gate on the claimed sender alone is a filter, not a boundary.
- `FileAttachment.size` is sender-declared too, and `BackfillStore::fetch_file` has no bound. Harmless while a person looks at the size and decides; not for an unattended download.
- Save notices go to the active channel (`push_system`) and `FileSaveFailed` carries no channel: right for `/save`, wrong for a file that arrives in a background channel (AC 5).
- A re-shared file is a new message, so message-id dedupe does not catch it, and a replayed share could start one download per copy. Folders and settings are keyed by stored name, which whoever shares a ticket chooses: a stale `on` would carry over to a different room that reuses the name.
2. Trust: one predicate, `AppState::is_trusted_peer`, used twice. Trusted is one of our own devices (`canonical_id == self_user_id`, i.e. certified under our user key; a forgotten device is not) or a peer we gave an alias (`petnames` holds their `canonical_id`). It is applied (a) to the message's claimed sender, to decide whether to try at all and to say why not, and (b) to the providers the file is fetched from. (b) is the boundary: peers are authenticated by the QUIC handshake and the blob by its hash, so bytes only ever come from a peer that passes, whoever the message claimed to be from. A forged "from your other device" share leads to asking the real devices, which do not have the blob, and fails.
3. Size: a fixed `files::AUTOSAVE_MAX_FILE_SIZE` (25 MB), separate from `/maxsize`, which limits what you share (500 MB to begin with, "only files you share are limited" -- LEY-25.8). Not configurable here: the ACs ask that a bare `/autosave` reports it, not that it can be changed. Checked twice: the declared size in app.rs (a cheap skip with a reason before any network), and the real size in `fetch_file`, which is what holds when the sender understates it. A blob already in the store is read with `blobs().status`; otherwise the providers are probed in order with `iroh_blobs::get::request::get_verified_size` (the last chunk and its proof, verified against the hash) before the downloader runs. Over the limit: nothing is downloaded and the claimed destination is removed by the existing failure cleanup.
4. `files::SaveMode { Manual, Automatic { max_bytes } }` is the one thing that differs between the two kinds of save: whether the size is bounded, and where the outcome is said (manual: the channel you are in, as today; automatic: the channel the file arrived in, wherever you are looking). `SaveRequest { channel, hash, filename, providers, dir, mode }` replaces the fields of `InputAction::SaveFile`, so main.rs's `start_save` (claim a name, hand over to `Net::save_file`) serves `/save`, the files overlay and auto-save alike. `NetEvent::FileSaved` and `FileSaveFailed` carry the mode; `FileSaveFailed` also gains the channel and hash. `AppState::save_failed` words every failure: `failed to save X: e` exactly as today for a manual save, `couldn't auto-save X: e -- /save <hash> to download it yourself` for an automatic one.
5. `/autosave [on|off]`. Bare says the state, who is accepted, the limit and the channel's folder; `on`/`off` act on the active channel (any kind), off by default, returning `InputAction::SetAutoSave { channel, on }` only when something changed. State is `AppState::autosave`, a set of stored names seeded by `load_autosave` and persisted by a new `src/autosave.rs` (a postcard list of stored names, mirroring `channel_renames.rs`; a corrupt file is logged and means off everywhere, the safe direction). The setting is forgotten on `/leave` (first in the leave sequence, so a crash midway leaves it off, never stale) and by pairing's wipe list. A write that fails is said with its consequence: an `off` that did not stick means on again after a restart.
6. `AppState::auto_save(channel, message, already_saved) -> Option<SaveRequest>`, called from the live arm in main.rs once the message is persisted. It ignores anything but a file from another device in a channel with auto-save on. It says why and stops when the sender is not trusted, the file is over the limit, or the channel already holds a saved copy (`SavedFiles::is_saved`, passed in as a closure so app.rs stays free of I/O). It drops a repeat of a save still in flight (`auto_saving`, cleared on success, failure and leave). Otherwise it builds the request: providers filtered by trust, into `channel_dir(root, channel)` of the channel the file arrived in. No "saving..." line; `auto-saved X to <path>` when it lands.
7. Docs and housekeeping: README (a `/autosave` row; the `/send` row no longer says "never automatically" flatly), features.md (an entry, with what it leaves out: unauthenticated sender, a fixed limit, no cumulative cap, device-certificate ordering), concept.md (File sharing, the data files, the layout), `/help`, `COMMAND_HINTS`. Cargo.toml 0.6.0 -> 0.7.0 (a feature: minor); Cargo.lock follows the build.
8. Tests. autosave.rs: missing/corrupt file, set/forget, independence, persistence. saved_files.rs: `is_saved`. backfill.rs (real loopback peers): over the limit downloads nothing and leaves the store without the blob, exactly at the limit passes, a blob already in the store is bounded too, an offline provider falls through to the next, nobody reachable says what it says today. app.rs: `/autosave` in each state and with bad arguments, per-channel independence, seeding, `remove_channel` forgets; `auto_save` for own device, aliased peer, unknown peer (with the notice and its channel), forgotten device, own message, no attachment, at and over the limit, already saved, in flight then finished or failed, providers filtered to the trusted (including a forged trusted sender with only a stranger online), notices landing in the file's channel while another is active; `FileSaved`/`save_failed` in each mode; `/help`. ui.rs: the hint. main.rs: the wipe-list test covers the new file.
9. Verify with cargo test, clippy --all-targets and fmt --check, then the real binary: instances under isolated HOMEs, each in a PTY read with pyte (scratch script outside the repo, run with uv). Off by default (a share while off is not saved and says nothing); a stranger's share is skipped with its reason; after `/alias` the next share lands in the channel folder with the notice and a same-named one becomes `name (1).ext`; a 26 MB share is skipped with its reason and `/save` still fetches it; enabling afterwards does not fetch what was shared before; after a restart the setting is still on and a file shared while B was away (backfilled) is not saved, while the next live one is; the notice shows up in a background channel's transcript, not the active one; and a paired sibling device is auto-saved with no alias.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Built as planned, with three small differences. (a) `BackfillStore::fetch_file` now asks the store once, with `blobs().status`, whether the file is held and how big it is, instead of `remote().local(..).is_complete()`: one query answers both, and the existing fetch tests pass unchanged. (b) The "paired sibling" in the real-binary run is a second instance given a copy of the first one's user key file, not one made with `/pair`: `/pair` writes the real clipboard, which the earlier PTY runs avoided, and that key is exactly what pairing hands over. (c) The "26 MB" file is 27,000,000 bytes (25.7 MB): over the 25.0 MB limit, far under `/maxsize`.

Changed: src/autosave.rs (new: `AutoSave`, a list of stored names), src/files.rs (`AUTOSAVE_MAX_FILE_SIZE`, `SaveMode`), src/saved_files.rs (`is_saved`), src/backfill.rs (`fetch_file` takes `max_bytes`; `verified_size`, `ensure_within`), src/net.rs (`FileSaved` and `FileSaveFailed` carry the mode, `FileSaveFailed` also its channel and hash; `Net::save_file` takes it), src/app.rs (`SaveRequest`, `InputAction::SaveFile(SaveRequest)` and `SetAutoSave`, `/autosave`, `auto_save`, `is_trusted_peer`, `save_failed`, `push_system_to`, `AppState::autosave` and `auto_saving`, `remove_channel` forgets both, `/help`), src/main.rs (`start_save`, the live-arm hook, `SetAutoSave`, `Leave` forgets the setting first, `FileSaveFailed` through `save_failed`, startup load and seed, pairing's wipe list), src/ui.rs (the hint), README, features.md, concept.md, Cargo.toml 0.6.0 -> 0.7.0 (a feature: minor; Cargo.lock follows the build).

Validation: cargo test 938 passed (51 new, was 887), cargo clippy --all-targets clean, cargo fmt --check flags only the old hunk in src/dm_registry.rs (left alone). The real binary, instances under isolated HOMEs each in a PTY read with pyte (scratch script outside the repo, /tmp/ley-26-1/run.py): 66 of 66 checks pass. With auto-save off (the default) a share is neither saved nor mentioned, and turning it on later does not fetch what was shared before. A stranger's share is skipped with `not auto-saving stranger.txt from <id>: ...` and its `/save <prefix>` hint. After `/alias` the next share lands in `<root>/general/` with `auto-saved ok.txt to <path>`; two namesakes become `dup.txt` and `dup (1).txt`. A 25.7 MB file is skipped (`over the 25.0 MB auto-save limit`) and a manual `/save` still fetches all of it. After a restart the setting is still on; a file shared while B was away came back through backfill and was not fetched, the next live one was. Leaving #general and rejoining it from the ticket starts with auto-save off (the entry is gone from the data dir), and the history refetched on rejoin is not downloaded. With another channel on screen the notice is in #general's transcript, not the one being looked at. A second instance holding the first one's user key, and never aliased, is auto-saved.

Not covered by that run: a forged sender (the real client cannot send one; the unit tests `a_share_forged_as_a_trusted_peer_still_cannot_reach_a_stranger` and `a_file_is_only_ever_asked_of_peers_we_trust` cover the provider list, and the backfill tests cover the size check against real loopback peers, as a client that understates a file's size cannot be made either), a failed write of the setting, and Windows.

Interpretations to confirm:
(1) Trusted means one of your own devices (certified under your user key; a device forgotten with `/forget-device` is not) or a peer you gave an alias: the decision on the open question. It is applied twice, because nothing signs a chat message: the claimed sender only decides whether to try (and to say why not), and the file is only ever asked of peers that are themselves trusted, so a forged "from your other device" share fetches nothing. LEY-9 and LEY-10 should feed `is_trusted_peer`.
(2) The limit is a fixed 25 MB (`files::AUTOSAVE_MAX_FILE_SIZE`), separate from `/maxsize` (what you share, 500 MB), not configurable here, and bare `/autosave` reports it. There is no cap on the total: many files within the limit still add up.
(3) The size is enforced on what the file is, not on what its sender said: skipped up front when announced as too big, and measured again, from the store or from a provider's verified size, before anything is downloaded.
(4) A manual save is unchanged and answered in the channel you are in; an automatic one is answered in the channel its file arrived in, wherever you are looking.
(5) A repeat is dropped, not saved again: a file the channel already holds (said, in the channel) and one still downloading (quietly). Neither limits how many different files arrive.
(6) The setting is per stored channel name, forgotten on `/leave` (first in the leave sequence) and by pairing's wipe. A corrupt file means off everywhere. A failed write of `/autosave on|off` is reported with what it means (an `off` that did not stick is on again after a restart); a failed write on leave is only logged.
(7) `/autosave` works in group channels, `/msg` DMs and `/dm` chats alike.
(8) A skipped file always gets a notice, in a channel that has auto-save on: one line per skipped file, however many strangers share. A channel with it off hears nothing.
(9) Cargo.toml 0.6.0 -> 0.7.0 (a feature: minor).

Observed, not changed: (a) `ChatMessage::sender` is unauthenticated on gossip and on `/dm` (neither `net::forward_events` nor `direct::receive_loop` checks it), so anyone in a channel can show up in the transcript as your other device or an aliased friend; signing messages is a wire change and a task of its own. (b) An alias is stored under the identity its peer had when it was given (`petnames` is keyed by `canonical_id`), and `NetEvent::Device` moves a nickname and a version across to the user id once a certificate arrives but not a petname -- so, as read from the code, an alias given to a device before its certificate arrived stops applying afterwards, for the name shown and now for auto-save too.

Reviewed by Jurgen on 2026-10-13: (5) and (8) accepted. (2) accepted for this task as built -- a fixed limit -- with an adjustable one split out as LEY-26.9 (device-wide, a size and never `off`; a per-channel limit is left for later). The other interpretations wait on his test of the feature.

Second real-binary run (scratch script /tmp/ley-26-1/run2.py, same harness), for the two clauses the first could not show: 18 of 18 checks pass. A fake `open` first on PATH is never called by auto-save and is called by Ctrl+O in `/files` (the positive control), so nothing is opened automatically. With a channel's folder made impossible (a plain file where it should be), the transcript says `couldn't auto-save blocked.txt: <path> already exists and isn't a folder -- ... -- /save <prefix> to download it yourself`, nothing is written, and once the way is clear the next share is saved and a share of the very file that failed is tried again, not stuck as in flight.

Tried by hand by Jurgen on 2026-10-13 and found to work; with that the interpretations above are accepted, the adjustable limit being LEY-26.9.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
`/autosave [on|off]` makes auto-save an explicit, per-channel opt-in: off by default, remembered across restarts, and forgotten when a channel is left or by pairing. With it on, a file that arrives live from one of your own devices or a peer you have aliased, up to 25 MB, is saved into the channel's folder through the same pipeline as `/save` (one `SaveRequest`, so the same sanitizing and `name (1).ext`), and the channel's own transcript says where it went, or why it was skipped or failed. Chat messages are not signed, so a file is only ever fetched from peers that are themselves trusted, and its real size is checked before anything is downloaded; nothing from history, backfill or from before auto-save was on is fetched, and nothing is ever opened. Verified with 938 unit tests (51 new), clippy, 66 of 66 and 18 of 18 scripted checks of real instances under PTYs (a fake `open` on PATH for the never-opened clause, a blocked folder for the failure notice), and by hand. Version 0.7.0; the adjustable limit is LEY-26.9.
<!-- SECTION:FINAL_SUMMARY:END -->
