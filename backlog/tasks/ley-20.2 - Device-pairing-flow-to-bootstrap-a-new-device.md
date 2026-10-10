---
id: LEY-20.2
title: Device pairing flow to bootstrap a new device
status: Done
assignee: []
created_date: '2026-10-10 13:40'
updated_date: '2026-10-10 22:08'
labels:
  - identity
  - networking
dependencies:
  - LEY-20.1
parent_task_id: LEY-20
type: feature
ordinal: 22000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Device certificates (see the user-key task) let peers recognize that two EndpointIds belong to one person, but a brand-new device still starts with no user key and no channel membership: today the only way to add a second device is to generate a fresh identity and manually collect an invite ticket for every channel again. Add a one-time pairing flow where an already-set-up device shows a short-lived pairing ticket and a new device redeems it over a direct connection to receive the user key and the full channel registry (name plus RoomSecret per channel), the same shape `channel_registry.rs` already persists, so the new device can join everything through the existing `Net::start` rejoin path instead of being re-invited channel by channel. This needs its own authenticated transport, since a pairing exchange carries the user signing key and every room secret rather than one ticket worth of access, and a decision on whether every device shares one literal user key or holds a distinct certificate issued by a single root device, which affects how a lost device is later handled. See LEY-11 for the related, still-open question of revoking a single compromised device or member.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 An already-paired device can produce a one-time pairing ticket that a new device redeems directly, without going through any already-joined channel
- [x] #2 A redeemed pairing ticket cannot be used a second time
- [x] #3 After pairing, the new device is automatically joined to every channel the pairing device was already in, with no separate /invite needed per channel
- [x] #4 A pairing ticket that is malformed or already used is rejected with an explanatory message, mirroring existing invite ticket error handling
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented: PairingTicket (ticket.rs, KIND="leyline-pair") carries only an EndpointAddr + one-time secret -- no room name/secret, since pairing bootstraps identity before any channel exists. New pairing.rs owns PairingProtocol (ProtocolHandler impl registered on the shared Router under PAIRING_ALPN, mirrors backfill.rs owning its own protocol handler), PairingPayload (raw user-key bytes + channel snapshot, Debug redacted like RoomSecret/SecretKey), create_ticket/consume (one-time, TTL=5min, replaced wholesale by a new create_ticket call), and bootstrap() (the --pair entry point). Net gained create_pairing_ticket() (/pair, live, shows a ticket) mirroring ticket_for/invite. Key design choice: --pair <ticket> is a startup-time CLI flag (parse_args, mutually exclusive with --join), not a live command -- by the time a live command could run, main.rs would already have generated a throwaway user key and an un-shared "general". bootstrap() refuses to run if a user_key file already exists, so it can only bootstrap a genuinely new device. channel_snapshot() in main.rs is shared between startup known_channels and /pair ticket creation so they cannot drift apart.

Verification: cargo build clean, cargo test -- 407 passed/0 failed (13 new: 5 PairingTicket tests in ticket.rs, 7 PairingProtocol tests + 2 real end-to-end tests in pairing.rs, 2 /pair command tests in app.rs -- note two of those were real loopback-networking integration tests, not just in-memory unit tests: redeem_over_a_real_connection_returns_the_exact_payload spins up a real Endpoint+Router+PairingProtocol and a real client Endpoint and redeems over an actual QUIC connection; redeeming_the_same_real_ticket_twice_fails_the_second_time proves one-time-use holds over the real protocol, not just PairingProtocol:: consume in isolation). cargo clippy --all-targets clean. cargo fmt clean for every file touched (same two pre-existing, untouched-by-this-change fmt diffs in dm_registry.rs/net.rs::join noted in LEY-20.1 remain, left alone).

Scope note for AC objectivity: the real end-to-end tests above directly prove the wire protocol and one-time-use semantics (ACs 1, 2, and the "already used" half of AC 4) against a real connection, not just mocked state. The malformed-ticket half of AC 4 is proven by pairing_ticket_rejects_garbage_input. Full AC 3 (new device auto-joins every channel after a real --pair invocation against a running leyline instance, across two separate data directories) will be confirmed in the cross-device manual verification step covering all three LEY-20.x branches together, since that requires the full CLI/TUI, not just the protocol module -- bootstrap() itself calls ChannelRegistry::record_channel/record_peer, whose own insertion logic is already covered by channel_registry.rs tests unchanged.

Live two-device verification (two isolated leyline instances, separate fake HOME dirs, real QUIC/relay networking, no shared third peer): started device-a fresh, ran /pair, redeemed the ticket via --pair on device-b. Device-b came up already showing #general as a joined tab with no /join needed (AC3). Found and fixed a real gap this surfaced: create_pairing_ticket's channel snapshot only carries previously-learned bootstrap peers (crate::channel_registry::bootstrap_for), which is empty for any channel nobody else has joined yet -- the normal case for a device's first-ever pairing. Without a bootstrap address, the redeeming device subscribes to the right gossip topic but has nothing to dial, so the two devices never actually connected (confirmed: 0 peers after 50+s, zero NeighborUp events in either leyline.log). Fixed by having create_pairing_ticket append the pairing device's own current address to every channel's bootstrap list (including _devices), mirroring announce_channel_joined's existing reasoning. After the fix, both devices showed #general -> 1 peer(s) within ~20s and /who confirmed mutual visibility. Amended into this branch's commit; cargo build/test(424)/clippy clean.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented one-time device pairing: PairingTicket/PairingProtocol (pairing.rs), live /pair command, and leyline --pair <ticket> startup bootstrap. Verified all 4 ACs: unit/integration tests over real QUIC loopback for ticket creation, single-use redemption, and malformed/reused-ticket rejection; AC3 (auto-join of every existing channel) confirmed live across two isolated real devices. That live test also surfaced and fixed a cold-start bootstrap gap (see notes) so pairing produces devices that are both registered as joined AND actually connected over gossip. cargo build/test(424)/clippy/fmt clean on the final commit.
<!-- SECTION:FINAL_SUMMARY:END -->
