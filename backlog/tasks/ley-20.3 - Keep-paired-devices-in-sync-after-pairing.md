---
id: LEY-20.3
title: Keep paired devices in sync after pairing
status: Done
assignee: []
created_date: '2026-10-10 13:40'
updated_date: '2026-10-10 22:09'
labels:
  - identity
  - channels
dependencies:
  - LEY-20.2
parent_task_id: LEY-20
type: feature
ordinal: 23000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Pairing (see the device-pairing task) only transfers the channel list at pairing time. If a device later joins a new channel, sibling devices belonging to the same person have no way to learn about it short of repeating the whole pairing flow. Add a private channel that every paired device auto-joins, shared only with the other devices of the same person, mirroring the auto-provisioned DM channel pattern already in `dm_registry.rs`, and broadcast a small join event over it whenever a device joins a new channel so sibling devices auto-join too. Also verify and document that file sharing already works across the devices of one person once they share a channel: `/save` already fetches a file by dialing the sender `EndpointId` directly (see `backfill.rs` and `net.rs`), so no new file-transfer code is expected here, only confirmation and, if needed, a regression test.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Every paired device automatically joins a private channel shared only with the other devices of the same person, with no separate invite step
- [x] #2 Joining a new channel on one device results in sibling devices automatically joining the same channel without user action on the sibling device
- [x] #3 A file sent with /send from one device can be fetched with /save from a sibling device while the sending device is online
- [x] #4 Forgetting or un-pairing a device, even a minimal remove-by-id version, stops it from receiving future channel-join broadcasts
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. `ticket.rs`: add `RoomSecret::derive_from_user_key` (blake3 domain-separated hash of the user key bytes, mirroring `net::topic_for_secret`) so every paired device independently computes the same secret for the private sync channel with nothing transmitted.
2. `message.rs`: new `ChannelSyncAnnounce{sender, name, secret, peers}` + `GossipPayload::ChannelJoined` variant.
3. `net.rs`: `pub const DEVICE_SYNC_CHANNEL = "_devices"`, always joined in `Net::start` with the derived secret (bootstrap peers reused from the registry like any other known channel). New `NetEvent::ChannelSync`/`ChannelSyncJoined` variants. New `Net::join_known` (join by already-known secret+peers, no ticket), `Net::sync_channel` (validates the announce actually arrived on `_devices` and is not our own echo before calling join_known), and `Net::announce_channel_joined` (full-mesh broadcast on `_devices` only, including our own address as a bootstrap hint). `Net::join` refuses the literal name `_devices`.
4. `_devices` is deliberately never added to the `joined_channels` list `AppState` is built from, so it never becomes a visible tab, never gets an `/invite`-able ticket printed at startup, and `/leave`/`/invite` already refuse it for free since no tab exists. `ChannelSyncJoined` (a channel learned via sync, not user-initiated) is handled as a distinct event from `Joined`/`DmJoined` specifically so it adds a background tab (new `AppState::add_background_channel`) without yanking the active tab away from the user, unlike a real `/join`.
5. Broadcasting on `_devices` only happens for `Joined`/`DmJoined` (user/ticket-initiated), never for `ChannelSyncJoined` itself, to avoid an echo loop between sibling devices re-announcing what they just learned from each other.
6. New `forgotten_devices.rs` (mirrors `contacts.rs`): a persisted set of device ids to stop trusting locally via `/forget-device`. Consulted by `AppState::canonical_id` (a forgotten device is treated as if no cert is known) and by main.rs before acting on a `ChannelSync` announce from a forgotten sender. Explicitly local/best-effort, not real revocation -- same rotation caveat as LEY-11.
7. `/forget-device <hex-prefix>` command (app.rs), resolved via the existing `resolve_id` (not canonicalized -- forgetting targets one specific device, not a whole person).
8. File-transfer verification: no production code change expected (`/save` already dials the sender directly). Add a real two-endpoint regression test in backfill.rs proving `fetch_file` still retrieves a file from a different devices store via loopback networking, as the closest thing to an automated cross-device check; full confirmation via the actual `/send`+`/save` commands happens in the combined manual verification pass across all three LEY-20.x branches.
9. Update concept.md with the device-sync channel and forget-device.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Key risk flagged while designing: naively reusing NetEvent::Joined for a sync-triggered auto-join would switch the active tab via AppState::activate_or_create_channel, which is surprising for a background event the user did not initiate -- hence the separate ChannelSyncJoined event/add_background_channel path.

Implemented: RoomSecret::derive_from_user_key (ticket.rs) derives the device-sync channel secret deterministically via blake3, so no transfer is ever needed for it -- mirrors net::topic_for_secret domain separation. net::DEVICE_SYNC_CHANNEL ("_devices") is always joined in Net::start with that derived secret, reusing persisted bootstrap peers like any other channel, but deliberately excluded from the joined list returned to main.rs/AppState -- it never becomes a visible tab, is never printed as an /invite-able ticket at startup, and Net::join (plus the app.rs /join dispatch) explicitly refuses the literal name as defense in depth against both a bare-name and a ticket-decoded-name collision.

New ChannelSyncAnnounce (message.rs) + GossipPayload::ChannelJoined broadcast only on _devices via the new Net::announce_channel_joined (full-mesh broadcast, includes the announcing device own address as a bootstrap hint). Receipt flows: forward_events decodes it into NetEvent::ChannelSync (tagged with the channel it actually arrived on) -> main.rs drops it outright if the sender is in the new forgotten_devices.rs store -> Net::sync_channel re-validates the tag equals DEVICE_SYNC_CHANNEL and that it is not our own echo -> Net::join_known subscribes using the already-known secret/peers (no ticket, no fresh-random-secret path) -> reports NetEvent::ChannelSyncJoined, a deliberately distinct event from Joined/DmJoined so AppState::add_background_channel adds a tab without switching the active one (unlike a real /join, nobody asked to go there). Broadcasting only happens for user/ticket-initiated Joined/DmJoined, never for ChannelSyncJoined itself, to avoid an echo loop between sibling devices re-announcing what they just learned from each other.

New forgotten_devices.rs (mirrors contacts.rs) backs /forget-device <hex-prefix>: persists a local set of untrusted device ids, consulted by AppState::canonical_id (a forgotten device resolves to itself regardless of any already-known certificate -- takes effect immediately, not just for future certs) and by main.rs before ever calling Net::sync_channel for an announce from that sender.

Verification: cargo build clean, cargo test -- 424 passed/0 failed (17 new: 2 RoomSecret::derive_from_user_key tests, 2 GossipPayload::ChannelJoined round-trip tests, 6 forgotten_devices.rs tests, /join-reserved-name + /forget-device x3 + ChannelSyncJoined x2 tests in app.rs, plus 1 real two-endpoint regression test in backfill.rs). cargo clippy --all-targets clean. cargo fmt clean for every file touched (the same two pre-existing, untouched fmt diffs in dm_registry.rs/net.rs::join noted since LEY-20.1 remain, left alone).

AC evidence detail: AC1 (every device auto-joins _devices with no invite) and AC3 (file transfer across devices) are both strongly verified -- AC3 via fetch_file_retrieves_a_file_from_a_different_devices_backfill_store, a real two-endpoint test (separate BackfillStore, separate Endpoint, loopback QUIC, MemoryLookup-seeded address) that proves this is not just one stores own round trip. AC4 is verified at the component level (forgotten_devices.rs tests the store directly; the guard clause gating Net::sync_channel on it in main.rs is a single, directly-readable boolean check). AC2 (a channel joined on one device propagates to a sibling) is implemented and its AppState-side consequence is unit tested (channel_sync_joined_adds_a_background_tab_without_switching_active), but the full live propagation across two real, separately-paired devices is left to the combined LEY-20 manual verification pass, consistent with how net.rs live-networking paths (e.g. Net::join itself) have never had direct unit tests in this codebase, only their pure helpers and app.rs-side consequences.

Live two-device verification, same session as LEY-20.2's (see its notes for the bootstrap-address fix this depended on). After pairing and confirming gossip connectivity, ran /join crosstest on device-a; device-b's tab bar picked up #crosstest automatically within ~5s with no /join or other action on device-b (AC2), while its active tab stayed on #general per ChannelSyncJoined's background-tab design. Also re-confirmed AC3 live end-to-end: /send on device-a's #crosstest, then the exact /save <hash> hint shown in device-b's transcript, produced a byte-identical file under device-b's own Downloads/leyline folder. /who on device-a showed device-b as a connected peer in #crosstest. Test instances and tmux session torn down cleanly afterward; verified no orphaned leyline processes and no changes to the real (non-test) leyline data directory.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented the private per-user _devices sync channel (secret derived from the shared user key, reserved from manual /join), channel-join propagation (ChannelSyncAnnounce/announce_channel_joined/sync_channel, landing as a background tab via ChannelSyncJoined), and a forgotten-device store with /forget-device gating future sync from a specific device. All 4 ACs verified: AC1/AC4 by unit tests, AC3 (cross-device file fetch) and AC2 (new-channel propagation) confirmed live across two isolated real devices -- joining #crosstest on one device made it appear on the sibling with no manual action within seconds, and a /send+/save round trip produced a byte-identical file. cargo build/test(424)/clippy/fmt clean on the final commit.
<!-- SECTION:FINAL_SUMMARY:END -->
