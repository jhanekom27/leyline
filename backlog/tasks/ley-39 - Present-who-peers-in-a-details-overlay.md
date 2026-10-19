---
id: LEY-39
title: Present /who peers in a details overlay
status: Done
assignee:
  - '@jurgen'
created_date: '2026-10-19 08:35'
updated_date: '2026-10-19 14:41'
labels:
  - ux
  - commands
dependencies: []
priority: medium
type: enhancement
ordinal: 82000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Replace the transcript-style /who output with a compact, dismissible peer-details overlay for the active channel.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Running /who opens an overlay that lists each active-channel peer in a table with endpoint ID, local alias, and broadcast nickname.
- [x] #2 Missing aliases or nicknames and long endpoint IDs remain readable without breaking the overlay layout.
- [x] #3 Escape dismisses the overlay and returns to the prior conversation state.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Include this device in the /who snapshot so the overlay is the complete endpoint reference for the active conversation. 2. Surface the locally configured broadcast nickname where available while keeping alias semantics explicit and retaining remote online/offline ordering. 3. Add focused state and renderer coverage for the local row, then rerun validation and record the correction.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented an app-owned /who peer-details overlay that snapshots each live peer in the active channel, preserving full raw endpoint IDs plus canonical local aliases and broadcast nicknames. The renderer uses a cleared responsive table with explicit missing-value cells and a dismissible empty state; Esc closes it, Ctrl+C still quits, and pasted text cannot reach the draft beneath it. Updated command hints, /help, and README. Validation: cargo fmt; cargo test (1,054 passed); cargo clippy; cargo build --release; git diff --check.

Reopened after review: /who should include remembered offline peers when their information is available and visually distinguish online and offline status like the peer sidebar.

Correction complete: /who now snapshots the same active-channel presence view as the sidebar, including remembered offline peers after online peers and applying canonical-device deduplication. The endpoint column shows a colored filled circle for online peers and a dim hollow circle for offline peers. Updated /help, command catalog wording, and README. Validation: cargo fmt; cargo test (1,055 passed); cargo clippy; cargo build --release; git diff --check.

Reopened after review: /who should include this device so one table contains every endpoint and available nickname or alias.

Correction complete: /who now inserts this device between online and remembered offline peers, marked as green '● you', so one table contains all endpoint IDs. AppState is seeded with the persisted local nickname at startup and /nick updates it immediately for the local row; aliases remain explicitly empty for this device. The endpoint column reserves room for the status marker while retaining full IDs on a 100-column terminal. Validation: cargo fmt; cargo test (1,056 passed); cargo clippy; cargo build --release; git diff --check.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced /who transcript notices with a dismissible complete peer-details overlay: this device, online peers, and remembered offline peers, with endpoint, alias, nickname, and presence information. Verified by focused app/UI coverage and the full 1,056-test suite.
<!-- SECTION:FINAL_SUMMARY:END -->
