---
id: LEY-37
title: Add release preparation skill and tag command
status: In Progress
assignee: []
created_date: '2026-10-19 01:58'
updated_date: '2026-10-19 03:21'
labels: []
dependencies: []
priority: medium
type: chore
ordinal: 80000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Add a repository skill that prepares a Leyline release from a feature branch and a Taskfile command that safely creates and pushes the matching release tag. Repair the current task release target so it no longer calls removed local publication tasks.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A repository skill proposes the next plain semver version from Cargo.toml, the latest release tag, and unreleased conventional commits, requiring user confirmation before changing files.
- [ ] #2 The skill creates a release branch, updates required release files, runs the release checks, commits and pushes the branch, and opens a pull request when gh authentication permits.
- [ ] #3 task git:tag verifies clean and current main, confirms the matching tag is unused, creates an annotated vX.Y.Z tag, and pushes only that tag to trigger cargo-dist.
- [ ] #4 task release completes without referring to removed release:draft or release:publish tasks.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented on branch chore/release-process:
- Added the confirmation-gated release-process skill and three release evaluation prompts.
- Repaired task release and added task git:tag, which validates main before creating and pushing only the matching annotated tag.
- Updated contributor and agent guidance for the new release flow.

Validation: eval JSON, cargo fmt, 1,045 tests, locked release build, cargo-dist generation/plan, Taskfile parsing, and safe git:tag failure from a feature branch all passed. The paired skill review is available at /tmp/release-process-skill-review.html.
<!-- SECTION:NOTES:END -->
