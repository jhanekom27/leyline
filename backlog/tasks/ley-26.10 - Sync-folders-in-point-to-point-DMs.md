---
id: LEY-26.10
title: Sync folders in point-to-point DMs
status: Done
assignee: []
created_date: '2026-10-13 14:22'
updated_date: '2026-10-14 05:47'
labels:
  - files
  - networking
dependencies:
  - LEY-26.6
parent_task_id: LEY-26
type: feature
ordinal: 56000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend shared-folder sync to /dm point-to-point conversations. Direct peers currently derive distinct local p2p channel names and independently create docs namespaces, so their folder indexes do not converge. Establish and persist one shared docs capability for the direct conversation through the direct protocol without weakening the existing no-ticket direct-message flow.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A /dm conversation has one shared persisted docs namespace on both peers without requiring an extra invite
- [x] #2 With /sync on on both direct peers, a stable file created in one direct-conversation folder appears once in the other
- [x] #3 Direct reconnects and normal message delivery remain stable while folder sync is active
- [x] #4 Existing ticket-backed group-channel sync behavior remains unchanged
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Extend DirectRegistry with an optional persisted iroh-docs capability and backward-compatible peer-only decoding. 2. Add a direct control frame for capability exchange, preserving existing ChatMessage delivery; choose the capability creator deterministically from endpoint IDs. 3. Join and leave the existing SyncIndex for local p2p channel names, exchange/replay the shared capability on direct connection, and persist updates through the main event loop. 4. Restore direct indexes at startup and keep the existing sync scan/apply path unchanged. 5. Add persistence and direct loopback coverage; run cargo fmt, cargo test, cargo clippy, and cargo build --release.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented direct docs-capability persistence and negotiation. DirectRegistry now stores optional capabilities with peer-only migration; direct QUIC adds isolated control frames for capabilities; Net deterministically creates a namespace only for the lower endpoint ID, imports it on the other side, and leaves it with the direct channel. Startup restores direct docs state and the existing sync scan/apply path now receives index events for p2p folders unchanged. Verification: cargo fmt; cargo test (966 passed); cargo clippy (2 pre-existing warnings: unused ChannelRegistry methods and Net::start argument count); cargo build --release; git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Direct-message folder sync now persists and exchanges a shared iroh-docs capability through the authenticated direct QUIC connection. A deterministic endpoint-ID owner creates the namespace; the other peer imports it, and both restore it at startup. The existing publish, remote-apply, conflict, echo-suppression, reconnect, and group-channel flows remain intact. Verified with a two-instance direct-message folder-sync test, cargo fmt, cargo test (966 passed), cargo clippy (two pre-existing warnings), cargo build --release, and git diff --check.
<!-- SECTION:FINAL_SUMMARY:END -->
