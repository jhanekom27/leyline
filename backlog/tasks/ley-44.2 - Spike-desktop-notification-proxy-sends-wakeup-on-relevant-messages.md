---
id: LEY-44.2
title: 'Spike: desktop notification proxy sends wakeup on relevant messages'
status: To Do
assignee: []
created_date: '2026-10-23 13:56'
updated_date: '2026-10-23 13:57'
labels:
  - mobile
  - notifications
dependencies:
  - LEY-44
  - LEY-44.1
references:
  - decision-4
  - LEY-44
parent_task_id: LEY-44
priority: high
type: spike
ordinal: 91000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Goal

On a running desktop (or any always-on peer), when a message arrives that should nudge another of the user's devices, POST an opaque wakeup to that device's notify webhook. Chat content must still move only over iroh.

## Notes

- Depends on LEY-44.1 (notify endpoint available to the notifier).
- Start with a simple relevance rule (e.g. any message in a joined channel, or only DMs) and document what a later mute/mention filter would hook into (LEY-4).
- Wakeup payload must not include message plaintext; channel id optional only if justified and documented.
- Dedup/rate-limit so a busy channel cannot hammer the webhook.
- Out of scope: real iOS/Android notification centers (see sibling spike); shipping APNs keys.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Given a configured notify URL for a self-device, receiving a relevant chat event on desktop causes a webhook wakeup within a small bound
- [ ] #2 Webhook body contains no message text or attachment bytes
- [ ] #3 Repeated messages are rate-limited or coalesced so one burst does not create unbounded POSTs
- [ ] #4 With no notify URL configured, desktop behavior is unchanged aside from an explicit status/log that paired notifications are inactive
- [ ] #5 Findings written back on this task
<!-- AC:END -->
