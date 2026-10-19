---
id: LEY-36
title: Establish pull-request CI and release automation
status: Done
assignee: []
created_date: '2026-10-19 01:00'
updated_date: '2026-10-19 06:53'
labels: []
dependencies: []
priority: high
type: chore
ordinal: 79000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Establish a trunk-based development workflow for the public leyline repository. Add GitHub Actions CI for pull requests and main, protect main from direct changes, document the contributor workflow, and define a release-time semver process compatible with the existing cargo-dist Taskfile release path.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Every pull request and push to main runs the required Rust format, test, lint, and release-build checks with locked dependencies.
- [x] #2 main requires a pull request, successful CI checks, and an approving review before merge; direct pushes are blocked.
- [x] #3 CONTRIBUTING.md documents feature branches, pull requests, local validation, and the release-only version-bump policy.
- [x] #4 Release automation validates that a vX.Y.Z tag on main matches Cargo.toml and produces the configured distribution artifacts and GitHub release.
- [x] #5 Documentation changes merged to main build, test, and deploy the guide to GitHub Pages through GitHub Actions.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented on branch chore/ley-36-ci-cd:
- Added Rust CI, cargo-dist-generated tag release workflow, and GitHub Pages documentation deployment workflow.
- Replaced local publication tasks with local verification only and documented protected PR, release-time versioning, and tag-triggered releases.
- Validated Cargo formatting, Clippy (five existing warnings), 1,045 tests, locked release build, mdBook build/test, workflow YAML parsing, and cargo-dist generation/plan.

Pending after merge: run the workflows on main, switch GitHub Pages to Actions deployment, and add the observed Rust CI check names to the protected-main ruleset.

Post-merge verification: main ruleset now requires one approving review, requires Rust quality and MSRV compatibility checks on an up-to-date branch, and grants jhanekom27 an always bypass. Release v0.21.1 completed successfully with all configured artifacts. GitHub Pages uses the Actions build type, and the latest main documentation deployment succeeded.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Established protected trunk-based delivery: verified Rust CI on main, one-review ruleset enforcement with owner bypass, successful v0.21.1 cargo-dist publication, and successful GitHub Pages deployment from Actions.
<!-- SECTION:FINAL_SUMMARY:END -->
