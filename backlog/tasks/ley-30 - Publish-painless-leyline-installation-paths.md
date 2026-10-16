---
id: LEY-30
title: Publish painless leyline installation paths
status: Done
assignee: []
created_date: '2026-10-15 01:43'
updated_date: '2026-10-16 06:51'
labels: []
dependencies: []
type: feature
ordinal: 71000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Make leyline practical to install for people who are not contributors. Add automated release binaries and document cargo installation options; evaluate a Homebrew tap as an optional distribution channel.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 GitHub Releases can publish prebuilt binaries for supported platforms through an automated release workflow
- [x] #2 README documents supported cargo install paths without requiring Backlog.md
- [x] #3 Distribution documentation clearly distinguishes end-user installation from contributor tooling
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented a local-only cargo-dist v0.32.0 release configuration and a single guarded task release workflow. Verified cargo-dist planning and all four archives/checksums with task release:build; cargo fmt --check, cargo test (1,007 passed), cargo clippy, task docs:build, and task docs:test pass. Acceptance criterion 1 remains unchecked until the workflow creates and publishes a real GitHub Release from a clean, pushed release commit; no release was created during implementation.

Published GitHub Release v0.18.0 from commit d85705b with four platform archives, four per-archive SHA-256 files, and sha256.sum. Verified the remote v0.18.0 tag, non-draft release state, nine uploaded assets, and a clean working tree via GitHub CLI.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added local cargo-dist release automation and installation documentation, then published v0.18.0 with macOS, Linux, and Windows archives. Verified task release builds, checksums, native version smoke check, Rust tests/lint, mdBook tests, remote tag, and public release assets.
<!-- SECTION:FINAL_SUMMARY:END -->
