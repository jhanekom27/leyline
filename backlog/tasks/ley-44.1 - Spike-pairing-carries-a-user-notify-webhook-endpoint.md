---
id: LEY-44.1
title: 'Spike: pairing carries a user notify webhook endpoint'
status: To Do
assignee: []
created_date: '2026-10-23 13:56'
labels:
  - mobile
  - notifications
  - pairing
dependencies:
  - LEY-44
references:
  - decision-4
  - LEY-44
  - LEY-20
parent_task_id: LEY-44
priority: high
type: spike
ordinal: 90000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Goal

Prove that a device can publish a user-configured doorbell URL (e.g. ntfy topic) through the existing pairing / multi-device trust path so other self-devices can wake it — without a Leyline-hosted service and without putting message bodies on the wire.

## Notes

- Prefer reusing pairing ticket / device cert identity (LEY-20) over inventing a second trust model.
- Store the endpoint as local settings plus whatever minimal cross-device distribution is needed for paired notifiers to learn it.
- Validate URL shape enough to fail closed on empty/garbage input; do not fetch arbitrary SSRF-prone targets beyond the configured doorbell POST.
- Out of scope: APNs/FCM tokens, mobile UI, mute rules beyond a stub.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A paired device can record a notify webhook URL associated with another self-device (or the user) after pairing or an explicit notify-setup command
- [ ] #2 The URL persists across restart on the devices that need it to send wakeups
- [ ] #3 No Leyline server is required; automated or manual test uses a local/mock HTTP collector or public ntfy topic under the tester's control
- [ ] #4 Findings (wire shape, storage location, open questions) are written back on this task
<!-- AC:END -->
