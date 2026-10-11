---
id: LEY-23
title: 'Recognize a paired sibling device as our own identity, not a stranger'
status: Done
assignee: []
created_date: '2026-10-11 13:57'
updated_date: '2026-10-11 13:58'
labels:
  - identity
  - ux
dependencies: []
type: bug
ordinal: 26000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
LEY-22 taught the peers sidebar to collapse two OTHER peers sharing a DeviceCert user_id into one row, but never taught AppState to recognize when a peer's canonical identity is actually OUR OWN user_id -- i.e. one of our own other paired devices. canonical_id/device_to_user only ever map a device id to a user id learned from a *received* certificate, and nothing ever sends us a certificate for our own device id (announce_device only ever broadcasts outward, there is no self-reception in gossip). So AppState has no notion of its own canonical user identity at all.

Confirmed live after pairing two real devices via LEY-21's /pair <ticket>: each side still showed the other as an unrelated stranger -- a second row in the peers sidebar (in addition to the dedicated 'you' row), and a raw hex id as the sender of a message/reply typed on the sibling device, exactly as reported (seeing 'the short id of myself' in the peer list, and 'the id of my other login' on replies).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 After two devices are paired, the peers sidebar on each one shows only the dedicated 'you' row -- the sibling device no longer appears as a second, separate entry
- [x] #2 A message or reply preview originated on a paired sibling device displays as 'you' in the transcript of any other device sharing that same user key
- [x] #3 /who's existing per-device technical listing (showing every connected device's own hex id, including a sibling's) is unaffected -- a deliberate, more detailed view, still useful for confirming which of your own devices are actually connected
- [x] #4 A petname manually /alias-ed for a sibling device no longer overrides the 'you' display, mirroring how a petname can never override the existing self_id 'you' check either
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Thread our own canonical user identity (the shared user key's public half) through to AppState, so it can compare a peer's resolved canonical_id against our own instead of only ever grouping two OTHER peers together.

net.rs: new Net::our_user_id(&self) -> [u8; 32] accessor, returning the already-computed our_cert.user_id (our_cert is built once in Net::start from the user_key passed in).

app.rs: AppState gains a private self_user_id: [u8; 32] field (defaults to self_id in AppState::new, which can never collide with a real peer's canonical id) plus a new pub fn load_self_user_id(&mut self, user_id: [u8; 32]), mirroring the existing load_contacts/load_settings/load_dm_registry/load_forgotten_devices pattern -- called once right after AppState::new, before the event loop starts. No change to AppState::new's signature, so none of the many existing test call sites needed updating.

display_name: after the existing self_id check, also returns 'you' when canonical_id(sender) == self_user_id -- before the petname/nickname/hex-prefix fallback chain, so recognizing our own identity always wins over a manually set petname, mirroring the self_id check's own precedence.

unique_peer_ids: now also filters out any id whose canonical_id equals self_user_id entirely (not merged into a group -- excluded outright), since that peer is already represented by the sidebar's separate 'you' row.

main.rs: app.load_self_user_id(net.our_user_id()) added right after AppState::new, alongside the other load_* calls.

/who and grouped_who_lines/who_line are untouched -- they intentionally keep showing every raw device id, including a sibling's, as a technical/diagnostic view.

Testing: new app.rs unit tests (alongside the existing canonical_id/unique_peer_ids tests) covering: display_name returns 'you' for a peer certified under our own loaded user key; that this wins over a petname set for the same canonical id; unique_peer_ids excludes such a peer entirely (not just merges it); and the empty-list case when a sibling device is the only peer. Also verified live: paired two real, separate devices via LEY-21's live /pair <ticket>, confirmed each side's peers sidebar shows only 'you' (not a second row) despite '1 peer(s)' being connected, and that a message typed on one device displays as 'you' in the other device's transcript.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Root cause confirmed: canonical_id/device_to_user only ever learn a mapping from a *received* DeviceCert, and nothing ever sends us a certificate for our own device id (announce_device only broadcasts outward, iroh-gossip has no self-reception) -- so AppState had literally no notion of its own canonical user identity, only the (already-working) ability to group two OTHER peers that both sent certs under the same user key.

Implemented exactly per plan: Net::our_user_id() accessor (reuses the already-computed our_cert.user_id); AppState::self_user_id private field (defaults to self_id, loaded via new load_self_user_id, mirroring load_contacts etc.); display_name and unique_peer_ids both now compare a peer's canonical_id against self_user_id. main.rs wires app.load_self_user_id(net.our_user_id()) in right after AppState::new. /who/grouped_who_lines/who_line deliberately untouched.

Verification: cargo build clean, cargo test -- 440 passed/0 failed (4 new: display_name shows 'you' for a certified sibling device; that this wins over a petname set for the same canonical id; unique_peer_ids excludes such a peer entirely rather than merging it; and the empty-result case when the only peer is a sibling device). cargo clippy --all-targets clean. cargo fmt clean for all touched files (same pre-existing, unrelated fmt diffs in dm_registry.rs/net.rs noted in every prior LEY-20.x/21/22 task remain, left alone).

Live two-device verification (two isolated real leyline instances, real QUIC/relay networking): paired device-b to device-a via LEY-21's live /pair <ticket> flow. Confirmed on both sides: the peers sidebar shows only the single '* you' row despite '#general * 1 peer(s)' being connected (previously showed a second row for the sibling's raw hex id). Sent a message from device-b and confirmed it displays as 'you' (not a hex id) in device-a's transcript. This also happened to re-confirm LEY-21's live pairing flow still works end to end.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
AppState now knows its own canonical user identity (Net::our_user_id, loaded via the new AppState::load_self_user_id, mirroring the existing load_contacts/load_settings pattern), so display_name and unique_peer_ids can recognize a peer certified under that same user key as one of our own other devices rather than an unrelated stranger -- the gap LEY-22's peer-to-peer-only grouping left open. /who's full technical listing is untouched. Verified all 4 ACs with new unit tests and a live two-device pairing test: the sidebar now shows only 'you' for a paired sibling device, and messages typed on it display as 'you' elsewhere. cargo build/test(440)/clippy/fmt clean.
<!-- SECTION:FINAL_SUMMARY:END -->
