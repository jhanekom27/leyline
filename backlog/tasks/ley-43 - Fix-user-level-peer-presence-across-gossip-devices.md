---
id: LEY-43
title: Fix user-level peer presence across gossip devices
status: In Progress
assignee: []
created_date: '2026-10-20 12:12'
updated_date: '2026-10-20 14:46'
labels:
  - identity
  - ux
  - networking
dependencies: []
priority: high
type: bug
ordinal: 87000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The peers sidebar currently treats only direct iroh-gossip neighbors as online. In a larger mesh, a channel member can exchange messages through relays while appearing offline. Device certificates are also announced only to direct neighbors, so a remote person's paired devices may not resolve to one canonical user and can render as duplicate rows. Represent presence at the certified user level in the sidebar: any active device keeps that person online. Restore /who as the separate per-device endpoint diagnostic view rather than deriving it from the sidebar's collapsed rows.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A remote channel member who is not a direct gossip neighbor appears online while this device receives their current authenticated device-presence announcements, and becomes offline after those announcements expire.
- [x] #2 Once certificates for two active devices share a user key, the peers sidebar renders one user row; that row stays online while either device remains active.
- [x] #3 A user who is only observed as an active remote device is canonicalized from a valid device certificate, without requiring a direct-neighbor event.
- [x] #4 /who shows the individual known device endpoints and their online/offline state, independently of the sidebar's user-level collapse.
- [x] #5 Focused app, network, and renderer tests cover direct and remote activity, expiry, device aggregation, raw /who rows, and legacy offline rows.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Use the existing signed GossipPayload::Device(DeviceCert) as a periodic, full-channel presence heartbeat rather than adding a second wire type. Change Net::announce_device in src/net.rs to use broadcast; continue sending it immediately when a direct neighbor connects, and call it for every visible joined channel from the existing 30-second history heartbeat in src/main.rs. The existing signature validation in forward_events remains the acceptance boundary for device identity.
2. Extend Channel and AppState with transient per-device activity observations, separate from persisted last_seen. A received valid device certificate marks that device active at the runtime’s observation time; direct neighbors remain immediately active through Channel::peers. Expire only certificate-derived activity after three missed 30-second heartbeats and make the existing 250ms tick prune expired entries and request a redraw. Record device-heartbeat observations in LastSeen too, so a remote person remains as an offline row after their active window expires or the app restarts.
3. Refactor presence derivation in src/app/identity.rs into two explicit views. The sidebar projection merges direct neighbors and fresh remote-device observations, groups them by canonical_id, excludes paired siblings of the local user, and keeps a user online while any represented device is active. The /who projection retains raw endpoint IDs from direct peers, fresh device activity, and persisted records, including paired sibling endpoints, and orders live rows before offline rows without canonical collapse.
4. Update run_who in src/app/commands.rs to snapshot the raw diagnostic presence projection. Keep src/ui/sidebar.rs unchanged in visual vocabulary but feed it the aggregated sidebar view; the peer overlay renders raw /who entries with individual status. Tag the local endpoint as you and paired sibling endpoints as mine so the diagnostic table distinguishes local devices from other users. Update concept.md’s Presence section to distinguish direct-neighbor events from full-channel, certificate-backed user presence and document the bounded eventual-consistency window.
5. Add focused coverage for remote activity, expiry, device aggregation, raw /who rows, the you/mine identifier, and legacy offline rows.
6. Run cargo fmt, cargo test, cargo clippy, and cargo build --release. Manually verify a multi-member channel where at least one active member is not a direct gossip neighbor, paired devices appear as one sidebar user while /who exposes both endpoint rows, and all of that user’s devices eventually become offline.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented the approved user-level presence path. Signed DeviceCert payloads now broadcast to the full channel both on NeighborUp and in the 30-second heartbeat. main.rs treats each valid received certificate as remote device activity and persists its last-seen timestamp; AppState expires that remote activity after 90 seconds while direct neighbors remain event-driven. The sidebar groups direct and fresh remote devices by canonical user identity, whereas /who now uses a raw per-device projection and includes paired siblings as diagnostic endpoints. Updated the Presence architecture documentation and device-certificate protocol comments. Automated validation passed: cargo fmt; cargo test (1,073 passed); cargo clippy --all-targets; cargo build --release; git diff --check. Clippy emitted only existing warnings: unused ChannelRegistry helpers, Net::start argument count, and three pre-existing owned-comparison warnings in app tests. Pending manual evidence: verify with a multi-member channel where an active member is not a direct gossip neighbor, then stop all of that user's devices and observe expiry.

Final automated verification after the last /who regression test: cargo test (1,074 passed) and git diff --check passed.

Added an explicit local-device marker in /who: the current endpoint renders as you and each paired sibling endpoint renders as mine, so raw endpoint IDs are attributable at a glance. Final automated validation passed: cargo fmt; cargo test (1,075 passed); cargo clippy --all-targets; cargo build --release; git diff --check. ACs 2 through 5 are verified by focused app and ratatui buffer tests. AC 1 remains open pending the documented live multi-member, non-direct-neighbor verification.
<!-- SECTION:NOTES:END -->
