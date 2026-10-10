---
id: decision-2
title: >-
  Room privacy via capability-style access, not name-derived topics or extra
  encryption
date: '2026-10-10 02:19'
status: accepted
---
## Context

A channel needs some way to gate who can join its gossip swarm. The simplest option would derive the gossip topic directly from the channel's name (e.g. a hash of it) -- but a name is typed by a human and often guessable or reused (`#general` being the obvious case), and it's shown in the UI and typed into `/join`, so it's effectively public. Deriving the topic from it would mean knowing or guessing a name is enough to join.

## Decision

A channel's gossip topic is `blake3("leyline-room:" + secret)`, where `secret` is a random 32 bytes generated the moment the channel is created (`RoomSecret::generate`, `ticket.rs`) -- never derived from the name or from anyone's identity, both of which are effectively public (concept.md's "Room privacy"). The secret travels only inside an invite ticket, shared once out-of-band. This is a capability-style access model, not extra encryption layered on top of what iroh already provides -- iroh's QUIC/TLS transport is end-to-end encrypted and authenticated regardless of any of this. The secret's only job is gating *who can attempt to join the swarm at all*: `iroh-gossip`'s HyParView membership layer accepts a `Join` from anyone who can reach an existing member, with no invite-list of its own, so an unguessable topic is the only thing standing in for one.

## Consequences

- Knowing or guessing a channel's name is never enough to join it -- you need a ticket, or to already be a recorded peer from a previous session.
- Two unrelated channels can share a display name without ever colliding on the same swarm, including `#general`: every fresh install generates its own private secret for it.
- The secret rides along in every invite ticket, so anyone already in a channel -- not just whoever created it -- can mint a new invite for it.
- There's no way to revoke a single member's access without rotating the whole channel's secret and re-inviting everyone else; see the open task documenting channel-secret rotation as the "remove a member" story.
- If a ticket names a channel already known locally under a different secret, `Net::join` rejects it with an explanatory message rather than silently switching that tab to a different swarm.

