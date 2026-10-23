---
id: LEY-44.3
title: 'Spike: wakeup consumer catch-up then local notification'
status: To Do
assignee: []
created_date: '2026-10-23 13:56'
updated_date: '2026-10-23 13:57'
labels:
  - mobile
  - notifications
dependencies:
  - LEY-44
  - LEY-44.2
references:
  - decision-4
  - LEY-44
parent_task_id: LEY-44
priority: medium
type: spike
ordinal: 92000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Goal

Prove the receive side of decision-4 without needing a full mobile UI: something acting as the \"phone\" receives a wakeup, reconnects/catches up over iroh as a real peer, then surfaces a local notification from local state.

## Notes

- May be a headless second Leyline instance, a small test harness, or an Android spike app — choose the smallest vehicle and document it.
- Demonstrates phone-as-peer (not SSH): identity + channel membership + backfill/live receive already exist on desktop path.
- Local notification can be OS-level on Android, or a clearly observable stub (desktop banner/log) if that unblocks faster; say which.
- Document iOS constraints: solo background without paired notifier remains best-effort.
- Out of scope: production mobile UI, folder sync, full command surface.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A consumer device can be woken by the doorbell from LEY-44.2 (or a manual webhook trigger standing in for it)
- [ ] #2 After wakeup, the consumer catches up to a message that arrived while it was "asleep" using iroh/Leyline paths only (no message body on the webhook)
- [ ] #3 A local notification or agreed stub fires from consumer-local state after catch-up
- [ ] #4 Write-up covers recommended mobile shell next step (Flutter/native/etc.) and background limitations
<!-- AC:END -->
