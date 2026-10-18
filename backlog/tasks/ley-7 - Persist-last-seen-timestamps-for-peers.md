---
id: LEY-7
title: Persist last-seen timestamps for peers
status: Done
assignee: []
created_date: '2026-10-08 02:11'
updated_date: '2026-10-18 13:49'
labels:
  - channels
dependencies: []
type: feature
ordinal: 7000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Presence is purely ephemeral today, driven only by live `NeighborUp`/`NeighborDown` gossip events (see concept.md's Presence section). A peer who goes offline simply disappears from the sidebar instead of being remembered.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The last time a peer was seen online is persisted per peer, per channel
- [x] #2 A peer who is not currently connected but has a recorded last-seen time is still listed, visually distinguished from online peers, instead of disappearing entirely
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add a small local `last_seen.rs` persistence module, following the existing postcard whole-file rewrite pattern used by `contacts.rs` and `channel_registry.rs`. Store a timestamped endpoint id per stored channel name; expose startup snapshot/lookup, touch-or-replace, and forget-channel operations. Missing or corrupt data starts empty, and removing a channel removes only that channel’s history. Cover persistence, update, per-channel isolation, and cleanup with inline module tests.

Extend `app::Channel` / `AppState` with per-channel remembered presence seeded at startup. Timestamp each `PeerJoined` observation in main.rs, persist it, and update app state with the same timestamp. A join refreshes that peer’s timestamp and marks it online; a leave removes it only from live `peers`, preserving the remembered record. Add a single derived peer-presence view that combines live peers and remembered offline peers, excludes our own canonical identity, and retains the current canonical-identity de-duplication semantics (using the newest timestamp if several devices resolve to one person). Keep all filesystem work out of `app`. Add focused app tests for join/leave retention, timestamp refresh, startup hydration, channel scoping, and canonical/self filtering.

Initialize `LastSeen` alongside the other local stores in `main.rs`, load its snapshot into `AppState` during startup, record every `PeerJoined` observation in the network-event branch, and forget the matching record when the existing `/leave` runtime path removes the channel. Log persistence failures consistently with the other best-effort local stores.

Change `ui::render_peers` to render the derived presence rows: live peers keep the current colored-dot presentation; remembered offline peers remain in the active channel’s sidebar with a visually muted offline marker and local last-seen time. Keep rendering read-only and use UI buffer tests to verify online/offline distinctions, no duplicate rows, and active-channel isolation.

Run `cargo fmt`, `cargo test`, `cargo clippy`, and `cargo build --release` because the change spans persistence, event routing, app state, and rendering.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented `last_seen.rs` as a local postcard store keyed by stored channel name and endpoint id. `main.rs` loads its snapshot into `AppState`, records every `PeerJoined` observation with one event-loop timestamp shared by state and storage, and forgets channel data during `/leave`. `AppState::peer_presence` combines live peers with remembered offline peers, collapses certified devices by canonical identity, and excludes paired sibling devices; `ui::render_peers` renders remembered peers as dim `○ <name> · last seen <local time>` rows.

Validation: `cargo fmt --check`, `cargo test` (1,039 passed), `cargo clippy`, and `cargo build --release` all completed. Clippy reports three existing warnings outside this work: unused channel-registry helpers, a `let...else` simplification in `src/app.rs`, and `Net::start` having eight parameters. Automated store, app-state, and ratatui buffer tests cover persistence, per-channel isolation and cleanup, offline retention, identity deduplication, and online/offline/sidebar rendering.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Persisted per-channel peer last-seen observations and retained offline peers in the sidebar with dimmed, timestamped rows. Verified with 1,039 passing tests, formatting, Clippy, and a successful release build.
<!-- SECTION:FINAL_SUMMARY:END -->
