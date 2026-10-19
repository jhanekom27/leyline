---
id: LEY-31
title: Complete leyline repository metadata and social preview
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-15 01:43'
updated_date: '2026-10-19 08:10'
labels: []
dependencies: []
type: chore
ordinal: 73000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Make leyline recognizable and discoverable on GitHub by configuring repository metadata, topics, licensing, and a social preview image.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Repository About information has a concise public description and relevant discovery topics
- [x] #2 The repository includes a clear license suitable for public distribution
- [x] #3 A social preview image is configured and renders appropriately for shared repository links
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Configure the public GitHub About description, homepage, and discovery topics to match leyline’s documented peer-to-peer terminal-chat behavior.
2. Add the license selected by the repository owner, including the repository file required for public distribution.
3. Create a social preview image that reflects leyline’s terminal-chat identity and configure it on GitHub.
4. Verify live repository metadata and preview configuration, then record the completed acceptance criteria and validation evidence.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Configured the live GitHub About section with the public description, guide homepage, and Rust/TUI/P2P/chat/file-sharing topics.

Added an MIT `LICENSE` and created `docs/assets/leyline-social-preview.png` (1280×640, 237 KB), with the editable SVG source beside it. GitHub’s public API cannot upload a repository social preview; it must be uploaded manually in repository Settings.

Validated the live metadata with `gh repo view`, confirmed GitHub is still using its default Open Graph image, and ran `git diff --check`.

Verified the uploaded social preview through GitHub GraphQL: `usesCustomOpenGraphImage` is true and GitHub returned the repository Open Graph image URL.

Acceptance criterion 2 remains in progress until the MIT LICENSE is merged to main.

After PR #8 merged to main, GitHub recognizes the MIT license on the repository. Reverified the live description, homepage, topics, and custom social preview through GitHub CLI and GraphQL.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Configured the public GitHub description, guide homepage, and discovery topics; added an MIT license; and activated a terminal-themed social preview. Verified after merge through GitHub repository metadata and GraphQL Open Graph image status.
<!-- SECTION:FINAL_SUMMARY:END -->
