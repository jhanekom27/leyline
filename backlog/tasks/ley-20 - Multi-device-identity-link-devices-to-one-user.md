---
id: LEY-20
title: 'Multi-device identity: link devices to one user'
status: Done
assignee: []
created_date: '2026-10-10 13:40'
updated_date: '2026-10-10 22:09'
labels:
  - identity
dependencies: []
type: feature
ordinal: 20000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Using leyline from a second device creates a second, totally unrelated identity: a fresh `EndpointId` with no connection to any other device. Every already-joined channel then needs a fresh invite, and peers see two strangers instead of one person. This epic tracks linking multiple devices to one persistent user identity, so a person is represented consistently to peers across every device they use, new devices inherit channel membership without being re-invited, and file sharing keeps working across the devices owned by one person. Related existing gaps: LEY-9 (out-of-band fingerprint verification) and LEY-11 (channel-secret rotation as the remove-a-member story) both touch the same trust model this work extends.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The devices owned by one person are represented as one identity to peers (display name, alias, presence, and /who), not as unrelated strangers
- [x] #2 Joining a channel on one already-paired device does not require a separate invite on another device owned by the same person
- [x] #3 A file sent with /send from one device remains fetchable with /save from another paired device owned by the same person
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Delivered via 3 stacked subtasks (branches device-identity -> device-pairing -> device-sync): LEY-20.1 bound device identities to a persistent, shared user key with signed device certs so peers group a person's devices as one identity (AC1); LEY-20.2 added one-time device pairing (/pair, leyline --pair <ticket>) so a new device inherits the shared identity and every existing channel with no manual re-invite; LEY-20.3 added the private _devices sync channel so joining a channel on one device propagates to paired siblings automatically (AC2) and confirmed cross-device file fetch keeps working (AC3). All three subtasks are Done with unit/integration test coverage plus a live two-device verification pass (separate real leyline instances, real QUIC/relay networking) exercising pairing, identity, channel auto-join, and /send+/save file transfer end-to-end. That live pass also found and fixed a cold-start gossip-bootstrap gap in pairing (see LEY-20.2 notes).
<!-- SECTION:FINAL_SUMMARY:END -->
