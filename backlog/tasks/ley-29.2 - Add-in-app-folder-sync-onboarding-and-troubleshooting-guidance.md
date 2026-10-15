---
id: LEY-29.2
title: Add in-app folder sync onboarding and troubleshooting guidance
status: Done
assignee: []
created_date: '2026-10-13 15:43'
updated_date: '2026-10-15 07:45'
labels:
  - files
  - ux
dependencies:
  - LEY-26.8
parent_task_id: LEY-29
type: docs
ordinal: 66000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Make sync discoverable from the terminal UI with concise first-use guidance and actionable troubleshooting for connection, permission, conflict, and size-limit failures.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 First-time /sync guidance explains the folder path and required invited channel setup
- [x] #2 Troubleshooting guidance gives actionable next steps for common sync errors
- [x] #3 In-app guidance stays consistent with README sync documentation
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
In run_sync (src/app.rs), show a concise first-use notice naming the folder path and setup appropriate to the channel kind: group channels use /invite and /join; /msg and /dm conversations ask both existing peers to enable sync. Preserve sync support for every channel kind. Extend sync_notice and runtime size-limit/permission-IO messages with actionable next steps pointing at /activity, /conflicts, /who, or folder permissions. Keep README and the mdBook guide aligned with recoverable delete/rename behavior and sync support across group and 1:1 channels. Add focused app.rs tests and validate with cargo fmt/test/clippy plus mdBook build/test.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Added a group-channel-only guard to /sync (DM and point-to-point chats are refused with a clear message), a first-use onboarding notice naming the folder path and /invite+/join setup, and actionable next-step hints: /conflicts for conflicts, /who for pending transfers, and a clarified fixed-size-limit message independent of /maxsize. Permission/IO-style sync errors now append a check-permissions-and-space hint. Updated README's /sync row and fixed a stale delete/rename claim in README and CONTRIBUTING to match the shipped recoverable-delete (sync trash) behavior. Added 8 new app.rs tests and updated 1 pre-existing test. Validated with cargo fmt, cargo test (1006 passed), cargo clippy (only pre-existing unrelated warnings), cargo build --release, and git diff --check.

Correction: an earlier implementation mistakenly treated stale LEY-29 documentation wording as a product constraint and briefly blocked /sync on DM/direct channels. LEY-26.10 and LEY-26.17 prove this is established functionality, so the restriction was removed. /sync now remains available on group, /msg, and /dm channels; only first-use wording varies by channel kind. Revalidated with cargo fmt, cargo test (1007 passed), cargo clippy (pre-existing unrelated warnings only), and task docs:build/test.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Made /sync more discoverable without restricting supported channels: first-use guidance now names each folder and gives group-specific invite/join or 1:1 peer-enable instructions. Added actionable conflict, pending-transfer, permission/IO, and fixed-size-limit hints; kept group, /msg, and /dm sync working; and aligned README and the mdBook guide with recoverable delete/rename behavior. Verified with 1,007 tests, fmt, clippy, and mdBook build/test.
<!-- SECTION:FINAL_SUMMARY:END -->
