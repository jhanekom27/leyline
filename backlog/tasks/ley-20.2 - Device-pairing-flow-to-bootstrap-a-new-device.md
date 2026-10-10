---
id: LEY-20.2
title: Device pairing flow to bootstrap a new device
status: To Do
assignee: []
created_date: '2026-10-10 13:40'
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
- [ ] #1 An already-paired device can produce a one-time pairing ticket that a new device redeems directly, without going through any already-joined channel
- [ ] #2 A redeemed pairing ticket cannot be used a second time
- [ ] #3 After pairing, the new device is automatically joined to every channel the pairing device was already in, with no separate /invite needed per channel
- [ ] #4 A pairing ticket that is malformed or already used is rejected with an explanatory message, mirroring existing invite ticket error handling
<!-- AC:END -->
