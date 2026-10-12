---
id: LEY-26.3
title: 'Spike: choose the substrate for two-way synced folders'
status: To Do
assignee: []
created_date: '2026-10-12 08:29'
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
- [ ] #1 A Backlog decision record compares iroh-docs with extending the manifest approach on: access control from the room secret, coexistence with the existing endpoint, gossip, and blob store, deletes and renames, offline catch-up, conflicts, storage growth, and maintenance risk
- [ ] #2 A throwaway prototype shows two instances with separate `$HOME`s converging on the same set of files after one adds, edits, and deletes a file
- [ ] #3 The record states whether a channel folder can be keyed from its room secret with no extra invite step, and what happens to the folder when the secret is rotated (LEY-11)
- [ ] #4 The record confirms or revises the provisional sync subtasks of this task
<!-- AC:END -->
