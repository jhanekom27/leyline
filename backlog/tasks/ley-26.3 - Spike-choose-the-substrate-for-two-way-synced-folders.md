---
id: LEY-26.3
title: 'Spike: choose the substrate for two-way synced folders'
status: Done
assignee: []
created_date: '2026-10-12 08:29'
updated_date: '2026-10-13 08:29'
labels:
  - files
  - networking
dependencies: []
parent_task_id: LEY-26
type: spike
ordinal: 43000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Two-way sync (adds, edits, deletes, renames, offline catch-up, conflicts) is a much larger commitment than attachments, so the substrate should be chosen deliberately. Candidates are iroh-docs, which depends on the same iroh, iroh-blobs, and iroh-gossip versions leyline already uses and replicates author-signed path to hash entries with set reconciliation, versus extending the existing hash-manifest approach used for history backfill (`backfill.rs`).

Key questions are how access can be derived from the channel room secret (as the private `_devices` channel already derives its secret from the user key), so the existing ticket stays the only credential (decision-2), how it coexists with the current router, gossip, and blob store, and what removing a member means (LEY-11).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A Backlog decision record compares iroh-docs with extending the manifest approach on: access control from the room secret, coexistence with the existing endpoint, gossip, and blob store, deletes and renames, offline catch-up, conflicts, storage growth, and maintenance risk
- [x] #2 A throwaway prototype shows two instances with separate `$HOME`s converging on the same set of files after one adds, edits, and deletes a file
- [x] #3 The record states whether a channel folder can be keyed from its room secret with no extra invite step, and what happens to the folder when the secret is rotated (LEY-11)
- [x] #4 The record confirms or revises the provisional sync subtasks of this task
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Prototype iroh-docs with two persistent peers on the project’s existing iroh/blobs/gossip versions; verify add, edit, and delete convergence. Compare it against extending HashSeq manifests in a decision record, including mutable paths, tombstones, authors, offline convergence, blob transfer, access control, and rotation. Persist the resulting namespace/access model and review all downstream shared-folder task scopes against it.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Verified prototype: cargo test --manifest-path /Users/jurgen/github/leyline-ley-26.3-spike/Cargo.toml sync_docs::docs_reconcile_add_edit_and_delete_between_persistent_peers passed (1 test, 0.37s). Two independent persistent iroh-docs peers converged after add, cross-peer edit, and delete. iroh-docs 0.101.0 compiles with leyline iroh 1.x, iroh-blobs 0.103, and iroh-gossip 0.101. The docs protocol shares the endpoint, blob store, gossip instance, and router under a dedicated ALPN. It provides signed multi-author entries, latest-per-key resolution, deletion tombstones, persistent replicas, range reconciliation, and blob transfer. Direct/local tickets require AddrInfoOptions::RelayAndAddresses instead of the default ID-only option.

Decision decision-3 selects iroh-docs over extending HashSeq manifests. HashSeq remains appropriate for immutable chat history but would require custom mutable paths, tombstones, multi-author conflict attribution, delta reconciliation, and content lifecycle management. iroh-docs supplies those as persistent signed CRDT entries backed by the existing blob store. Access model: each channel gets an independent persisted docs namespace and write capability; do not derive it from RoomSecret. Distribute the capability only through leyline channel-ticket and pairing flows, guarded by existing room-secret membership. Rotation is a new namespace plus entry migration and reinvitation, not in-place replacement of the gossip RoomSecret.

Final validation passed: cargo fmt, cargo test (946 passed), cargo clippy, cargo build --release, and git diff --check. The dedicated persistent two-peer docs test proved add, edit, and delete convergence.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Selected iroh-docs for per-channel shared-folder synchronization, verified with a persistent two-peer add/edit/delete convergence test, recorded decision-3, and propagated index, access, tombstone, and runtime boundaries to LEY-26.4 through LEY-26.8.
<!-- SECTION:FINAL_SUMMARY:END -->
