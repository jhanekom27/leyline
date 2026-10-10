---
id: LEY-20.1
title: Bind device identities to a persistent user key
status: Done
assignee: []
created_date: '2026-10-10 13:40'
updated_date: '2026-10-10 14:21'
labels:
  - identity
dependencies: []
parent_task_id: LEY-20
type: feature
ordinal: 21000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Today `identity.rs` persists one `iroh::SecretKey` per install, and every place that shows or stores a peer (`display_name`, `who_line`, `contacts.rs`, `dm_registry.rs`) keys off that single `EndpointId`, with no notion that two different `EndpointId`s could belong to the same person. Introduce a separate, longer-lived user key that signs a certificate binding a device `EndpointId` to it, broadcast the certificate over gossip the same way `IdentityAnnounce` and `VersionAnnounce` already are, and use the result so a peer who has seen certificates from two different `EndpointId`s signed by the same user key can display them as one identity. This is the foundation the pairing and device-sync tasks build on, and intentionally ships before any pairing transport exists: a second device can start out by manually copying the user key file.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A device generates or loads a persistent user key that is independent of its own per-device EndpointId
- [x] #2 A device signs and broadcasts a certificate binding its EndpointId to its user key, and re-announces it when a channel gains a neighbor the same way Identity and Version announcements already do
- [x] #3 A peer that receives certificates from two different EndpointIds signed by the same user key shows them as one identity in /who and display_name instead of two unrelated peers
- [x] #4 A peer with no certificate, because it is an older build or an unpaired device, still displays exactly as it does today
- [x] #5 Decoding a message from a peer that predates certificates does not error, matching the existing versioned-decode pattern in message.rs and ticket.rs
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Extract a reusable `persist(path, &SecretKey)` out of `identity.rs::generate_and_persist`, used by both key generation and (later) pairing.
2. Load a second persistent key in main.rs via the existing `identity::load_or_generate`, at `dirs.config_dir().join("user_key")` -- no new persistence code needed.
3. Add `DeviceCert { device_id, user_id, issued_at_unix_ms, signature }` to message.rs: sign/verify over the postcard bytes of (device_id, user_id, issued_at_unix_ms) using the iroh SecretKey/PublicKey the project already depends on. Add `GossipPayload::Device(DeviceCert)`.
4. `Net::start` takes the user SecretKey, computes our own DeviceCert once, and gains `announce_device(channel)` broadcasting it via broadcast_neighbors on NeighborUp, mirroring announce_nickname/announce_version exactly.
5. In app.rs, verify incoming DeviceCerts (drop+warn on bad signature), record valid ones in a device_to_user map, and add canonical_id() that resolves a sender to its user id when a cert is known, else itself unchanged. Route display_name, who_line, and the petname/nickname lookups through canonical_id so multi-device grouping is additive and a certless peer is unaffected.
6. Update concept.md Identity & channels section.
7. Unit tests: cert sign/verify round trip (including tampered signature and wrong user id rejection), GossipPayload::Device postcard round trip, canonical_id resolution, and that decoding a pre-cert message still succeeds.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Starting from the approved Warp plan (multi-device identity: device certs, pairing, and device sync).

Implemented: identity.rs::persist extracted and reused for the new user key (main.rs loads it at dirs.config_dir().join("user_key") alongside the existing device identity). DeviceCert{device_id,user_id,issued_at_unix_ms,signature} added to message.rs, signed/verified via iroh::SecretKey::sign/PublicKey::verify over postcard-encoded (device_id,user_id,issued_at_unix_ms) -- no new crypto dependency needed. GossipPayload::Device appended as the last enum variant (preserves existing variants postcard tags). Net::start takes the user key, computes our_cert once, and announce_device() mirrors announce_nickname/announce_version exactly, called from main.rs alongside those on PeerJoined. net::forward_events verifies DeviceCert::is_valid() at the network boundary before ever emitting NetEvent::Device, so app.rs only ever sees already-trusted certs. AppState::canonical_id resolves a device id to its user id when a verified cert is known, else the id unchanged; display_name, who_line grouping (new grouped_who_lines), and /alias (run_alias) all resolve through it. A nickname/version seen before its certificate (gossip has no delivery ordering) is migrated onto the canonical id once the cert arrives, so it is not orphaned. Scope decision: petnames/nicknames get live migration (in-memory only), but an alias set before a cert is known is not retroactively migrated into contacts.rs-persisted storage -- only alias going forward resolves canonically -- to avoid mutating persisted contacts from inside the pure app.rs event-handling layer.

Verification: cargo build clean, cargo test -- 394 passed/0 failed, cargo clippy --all-targets clean, cargo fmt clean for all files touched (two pre-existing fmt diffs in dm_registry.rs and net.rs::join, unrelated to this change and not touched, remain and were left alone). New tests: message.rs has 6 DeviceCert/GossipPayload::Device tests (sign/verify round trip, tampered device_id rejected, wrong-signer rejected, two devices share one user_id, postcard round trip, variant distinguishability). app.rs has canonical_id_is_unchanged_when_no_cert_is_known, a_peer_with_no_certificate_displays_exactly_as_before, device_event_makes_canonical_id_resolve_to_the_user_id, two_certified_devices_share_a_display_name, device_event_migrates_a_nickname_seen_before_the_certificate, alias_applies_to_a_second_device_once_its_certificate_is_known, who_groups_two_devices_certified_under_the_same_user_key -- plus every pre-existing /who, display_name, and /alias test still passes unchanged, proving a certless peer is unaffected. identity.rs has persist_writes_a_key_that_load_or_generate_reads_back.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a user key independent of each device identity, a self-signed DeviceCert binding device to user key, broadcast/verification over gossip, and canonical-id resolution so devices sharing a user key collapse into one display identity in /who, display_name, and /alias. Verified via cargo build/test/clippy/fmt and 13 new unit tests; no regressions in the 394-test suite.
<!-- SECTION:FINAL_SUMMARY:END -->
