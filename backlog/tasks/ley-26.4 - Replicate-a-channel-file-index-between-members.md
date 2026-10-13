---
id: LEY-26.4
title: Replicate a channel file index between members
status: Done
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 10:11'
labels:
  - files
  - networking
dependencies:
  - LEY-26.3
parent_task_id: LEY-26
type: feature
ordinal: 44000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Provisional until the spike lands. Every member of a channel needs to converge on the same view of which files exist in its folder (path, content hash, size, modified time, author) with no server, including peers that were offline. This task replicates that index only and does not touch the disk.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Each channel has a replicated file index that only holders of the channel ticket can read or write, with no extra invite step
- [x] #2 An entry added on one instance appears on the other members of the channel without user action, and a peer that was offline converges when it reconnects
- [x] #3 Index state persists across restarts and is removed when the channel is left
- [x] #4 Existing channels, DMs, and history backfill behave exactly as before
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Implement a persistent per-channel iroh-docs index service, extend registry/tickets/pairing with the docs capability, register docs on the existing endpoint/blob/gossip router, and bridge index events through the runtime without filesystem application. Add registry/ticket/pairing and two-peer offline-convergence tests, then run the full Rust validation set.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
LEY-26.3 substrate decision: implement the replicated index as one persisted iroh-docs namespace per channel. Store path-keyed entries with content hash, size, modified time, and author metadata; use per-device docs authors and Query::single_latest_per_key for the resolved index. Persist the docs namespace/write capability with channel state, distribute it only via channel ticket/pairing flows, and route docs live events into the runtime event loop.

Implemented persistent per-channel iroh-docs namespaces and capabilities, legacy-safe channel registry/ticket/pairing propagation, shared endpoint/blob/gossip/router registration, runtime-only index events, and channel-leave cleanup. Verified live convergence plus offline/restart catch-up. Final validation: cargo fmt, cargo test (947 passed), cargo clippy, cargo build --release, and git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added the persistent replicated per-channel file index using iroh-docs, automatically propagated its capability through invite and pairing flows, and verified live/offline convergence without filesystem application.
<!-- SECTION:FINAL_SUMMARY:END -->
