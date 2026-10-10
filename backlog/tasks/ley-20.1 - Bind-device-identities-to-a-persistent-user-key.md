---
id: LEY-20.1
title: Bind device identities to a persistent user key
status: To Do
assignee: []
created_date: '2026-10-10 13:40'
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
- [ ] #1 A device generates or loads a persistent user key that is independent of its own per-device EndpointId
- [ ] #2 A device signs and broadcasts a certificate binding its EndpointId to its user key, and re-announces it when a channel gains a neighbor the same way Identity and Version announcements already do
- [ ] #3 A peer that receives certificates from two different EndpointIds signed by the same user key shows them as one identity in /who and display_name instead of two unrelated peers
- [ ] #4 A peer with no certificate, because it is an older build or an unpaired device, still displays exactly as it does today
- [ ] #5 Decoding a message from a peer that predates certificates does not error, matching the existing versioned-decode pattern in message.rs and ticket.rs
<!-- AC:END -->
