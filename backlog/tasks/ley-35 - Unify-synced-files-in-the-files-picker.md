---
id: LEY-35
title: Unify synced files in the /files picker
status: Done
assignee: []
created_date: '2026-10-18 01:12'
updated_date: '2026-10-18 01:44'
labels: []
dependencies: []
priority: medium
type: feature
ordinal: 78000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Extend the existing /files fuzzy-search picker so it presents every file associated with the current channel in one place: files received through /send as well as files published through /sync. Preserve the current saved/autosaved status indicators for received files, and make synced files discoverable through the same Ctrl+P → channel → Ctrl+K workflow.

From a selected result, provide file actions appropriate to the local file: reveal it in the file manager, open it with the system-default application, or edit it with the system-default editor. The picker should make locating a specific channel file fast without requiring users to remember whether it arrived via /send or /sync.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 In a channel containing both received (/send) and synced (/sync) files, /files shows both kinds in one fuzzy-searchable picker.
- [x] #2 Existing saved and autosaved indicators for received files remain accurate in the unified picker.
- [x] #3 A user can search for a synced file by name after navigating to its channel and opening the picker.
- [x] #4 For a selected locally available file, the picker exposes actions to reveal it in the file manager, open it with the system default application, and edit it with the system default editor.
- [x] #5 Files unavailable locally are represented or handled clearly; no action falsely claims they were opened or revealed.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
# LEY-35: Unify synced files in the `/files` picker
## Problem
`/files` currently searches only attachment metadata from `/send`, even though `/sync` puts useful local files in the same channel folder. Users should be able to jump to a channel, open one file browser, fuzzy-search both sources, and act on an available local copy.
## Current state
`AppState::run_files` initializes a modal whose list is derived only from `shared_files`; the UI shows the saved checkmark and offers save or default-open actions (`src/app/files_overlay.rs:7`, `src/ui/files_overlay.rs:5`). The runtime already owns the channel folder and all I/O (`src/main.rs:1113`), while `sync_publish::scan` safely inventories the regular, non-hidden, non-temporary, non-outbox files that `/sync` considers (`src/sync_publish.rs:24`). Existing default-open behavior deliberately blocks executable, installer, script, shortcut, and launcher extensions (`src/files.rs:359`).
## Proposed changes
1. Add a runtime-owned, asynchronous channel-folder inventory request and result path for `/files`. When the overlay opens, scan its fixed channel folder on the blocking pool using the same safe recursive eligibility rules as sync; reject symlinks and omit hidden paths, `.leyline-trash`, the `send/` outbox, and temporary files. Correlate each result with the opening request and channel before applying it, so a late response cannot populate a closed or different overlay.
2. Replace the overlay’s attachment-only derived list with a unified owned entry model in `src/app/files_overlay.rs`. Preserve attachment rows and their sender/timestamp/hash so unsaved `/send` files can still use `Enter` to make the existing `/save` request. Add local-folder rows with relative path, absolute local path, size, and filesystem modification time. Merge a scanned path with an attachment’s recorded saved path to avoid duplicate rows and retain its current saved/autosaved checkmark; otherwise classify it as a sync-local entry. Sort the combined results consistently by their available timestamp and match attachment names/senders plus local relative paths case-insensitively.
3. Make actions depend on local availability instead of attachment origin. Keep `Enter` as the existing save action for an unsaved shared attachment. Add `Ctrl+O` to open any safe local copy with its default application, `Ctrl+R` to reveal its containing location, and `Ctrl+E` to open the local copy in the platform text editor. For unavailable attachment rows, surface a precise overlay notice instead of triggering an action; keep the current refusal to default-open executable or otherwise runnable files. Close the overlay before emitting an action so status and errors appear in the transcript.
4. Extend `src/files.rs` with shell-free, platform-specific reveal and editor launchers, using the same closed-stdio and child-reaping pattern as `open_in_default_app`. On macOS use Finder reveal and the system text editor; provide documented native fallbacks for Windows and Unix-like systems. Extend `InputAction` and the `main.rs` event loop to validate that a requested path is still a regular file under the channel folder before launching it, and report a clear failure when it moved or disappeared.
5. Update `src/ui/files_overlay.rs` to render mixed-source results without losing the saved checkmark, use a source/status indicator for sync-local rows, display relative paths, and make the footer advertise only actions valid for the selected entry. Preserve its existing filtering, keyboard navigation, narrow-terminal behavior, and no-results states.
6. Update all command-discovery surfaces to describe the expanded `/files` behavior: the command catalog and `/help` in `src/app.rs`, the Commands and keybinding documentation in `README.md`, the file-browser feature description in `features.md`, and the folder-sync guide in `guide/src/folder-sync.md`. Bump `Cargo.toml:3` from `0.19.0` to the next minor version for this feature.
## Validation
Add focused tests for the sync-compatible folder inventory, stale-result rejection, merged/deduplicated row construction, fuzzy matching of nested sync paths, saved/autosaved checkmarks, unavailable-file notices, and each modal key’s emitted action. Add renderer coverage for source indicators, actions, counts, and tiny terminals; add launcher-command tests that verify untrusted filenames remain a single command argument. Run `cargo fmt`, `cargo test`, `cargo clippy`, and a two-instance manual check: create nested synced files and `/send` attachments, confirm one picker can find both after `Ctrl+P` and `Ctrl+K`, then verify open, reveal, edit, missing-file, and runnable-file behavior.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented a background, sync-compatible channel-folder scan merged with attachment rows; saved paths deduplicate to preserve ✓ indicators. Added Ctrl+O open, Ctrl+R reveal, and Ctrl+E edit actions with regular-file revalidation; runnable files remain blocked from default-open. Validation: cargo fmt; cargo test (1,028 passed); cargo clippy (three existing warnings); cargo build --release succeeded.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Unified /files browsing for shared attachments and local sync files, with safe open/reveal/edit actions and updated documentation. Verified by focused inline coverage plus cargo test (1,028 passing), cargo clippy, and a release build.
<!-- SECTION:FINAL_SUMMARY:END -->
