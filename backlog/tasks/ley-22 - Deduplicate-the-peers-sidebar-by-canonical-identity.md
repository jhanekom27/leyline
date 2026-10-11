---
id: LEY-22
title: Deduplicate the peers sidebar by canonical identity
status: Done
assignee: []
created_date: '2026-10-11 06:34'
updated_date: '2026-10-11 07:01'
labels:
  - identity
  - ux
dependencies: []
type: feature
ordinal: 25000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The peers sidebar (ui::render_peers) lists one row per raw endpoint id currently in the active channel, so two devices paired to the same person (see LEY-20.1's device certificates) show up as two separate rows even though /who already groups them under one canonical identity (AppState::canonical_id / grouped_who_lines). Reuse that same grouping in the sidebar so it shows one row per person instead of one per device.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Once two devices are certified under the same user key (DeviceCert), the peers sidebar shows a single row for that person instead of one row per device
- [x] #2 A peer with no known device certificate still displays exactly as it does today -- one row, unaffected
- [x] #3 /who's existing per-device detail and (N devices) grouping is unchanged
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
AppState gains a small helper alongside canonical_id/grouped_who_lines: pub fn unique_peer_ids(&self, ids: &[[u8; 32]]) -> Vec<[u8; 32]>. Returns one id per distinct canonical_id(..) -- the first device seen for each, in original order -- the same grouping /who already applies, just without the (N devices) breakdown text that /who still exclusively owns. ui::render_peers calls this on channel.peers before building rows, so two certified devices collapse to one sidebar row (whichever's display_name -- same for both once certified); /who itself is untouched.

Testing: unit tests on unique_peer_ids (app.rs, alongside the existing canonical_id/display_name tests): two devices certified under the same user key collapse to one id; uncertified peers are each kept; original order is preserved.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as AppState::unique_peer_ids(&self, ids: &[[u8;32]]) -> Vec<[u8;32]>, placed alongside canonical_id: dedupes by canonical_id, keeping the first device seen per distinct person and preserving relative order (a HashSet of seen canonical ids + filter, mirroring grouped_who_lines's grouping without its '(N devices)' text). ui::render_peers calls it on channel.peers before building rows; /who and grouped_who_lines are untouched.

Verification: cargo build clean, cargo test -- 436 passed/0 failed (3 new: two certified devices collapse to one id, uncertified peers stay distinct, and a mixed case with one certified pair plus one uncertified third peer preserves original relative order). cargo clippy --all-targets clean. cargo fmt clean for both touched files (same two pre-existing, untouched-by-this-change fmt diffs in dm_registry.rs/net.rs noted in earlier LEY-20.x/LEY-21 tasks remain, left alone). Not separately live-tested: this reuses the exact canonical_id grouping LEY-20.1 already verified live and over unit tests; observing the sidebar collapse visually would need a third device's view of two already-paired devices, which doesn't exercise any new logic beyond what unique_peer_ids's unit tests already cover directly.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added AppState::unique_peer_ids, reusing the existing canonical_id grouping to collapse a channel's peer list to one entry per person, and wired it into ui::render_peers so the sidebar no longer shows a certified person's devices as separate rows. /who's own, more detailed grouping is unchanged. Verified all 3 ACs with new unit tests. cargo build/test(436)/clippy/fmt clean.
<!-- SECTION:FINAL_SUMMARY:END -->
