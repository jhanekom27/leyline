---
id: decision-1
title: 'Pure peer-to-peer messaging over iroh-gossip, no central server'
date: '2026-10-10 01:39'
status: proposed
---
## Context

leyline's pitch is "no servers, no accounts, no central relay to run" (README.md, concept.md's Goals). The default architecture for a chat app is client-server (or at least a lightweight relay), which makes message ordering, presence, and history straightforward -- but it also means someone has to run, pay for, and be trusted to operate that server, and every user needs an account on it.

## Decision

Messages travel directly between peers over `iroh` + `iroh-gossip` (concept.md's "Architecture at a glance"): a chat "channel" is a gossip topic peers join directly, with no server relaying or storing messages on anyone's behalf. Identity is a local keypair (`identity.rs`), not a server-issued account. The only external dependency is n0's public relay/discovery services, used solely as a NAT punch-through fallback when two peers can't connect directly -- they never see plaintext (iroh's QUIC/TLS transport is end-to-end encrypted) and never store anything.

## Consequences

- No infrastructure to run, pay for, or trust, and no central point of failure or censorship for an existing channel's membership.
- Delivery is best-effort and unordered across peers -- `ChatMessage`/`GossipPayload` dedupe by `id` and sort by `ts_unix_ms` rather than assuming arrival order (concept.md's "Message wire format").
- Presence (`NeighborUp`/`NeighborDown`) is only ever eventually consistent (concept.md's "Presence") -- there's no authoritative membership list to query.
- A peer offline while messages were sent needs a separate backfill mechanism (`backfill.rs`'s `iroh-blobs` manifests, see decision on history backfill) rather than a server simply replaying history on reconnect.
- Bootstrapping a new peer into a channel needs one piece of manual, out-of-band sharing (an invite ticket) since there's no directory service to look peers up through.

