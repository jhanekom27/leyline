---
id: LEY-44
title: 'Mobile Leyline: first-class peer with paired-device notification proxy'
status: To Do
assignee: []
created_date: '2026-10-23 13:56'
updated_date: '2026-10-23 13:57'
labels:
  - mobile
  - notifications
  - architecture
dependencies: []
references:
  - decision-4
  - concept.md
  - LEY-20
  - LEY-13
  - LEY-4
priority: high
type: feature
ordinal: 89000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
## Summary

Ship Leyline on phones as a real peer (own identity, channels, history when running), not a terminal remote shell. Reliable lock-screen notifications are an optional guarantee that appears only when another of the user's paired devices is online and can send a wakeup doorbell. Solo phone mode is best-effort. No Leyline-hosted chat server or message relay.

## Why

- SSH/TUI-on-phone cannot provide background notifications.
- Hosted chat infra would undo the iroh/no-server appeal.
- Multi-device pairing (LEY-20) already models one user across devices; mobile should reuse that.
- APNs/FCM (or a user-chosen webhook such as ntfy) may be used only as a platform doorbell among the user's own devices — never as the message path.

## Scope (product contract)

See decision-4. Phone = first-class peer. Paired online self-device = notification proxy (wakeup only; phone reconnects over iroh and catches up). No paired online device = degraded notifications with explicit status. v1 doorbell = user-configured webhook/ntfy URL via pairing.

## Out of scope for early spikes

- App Store / Play distribution polish
- Full slash-command parity on mobile UI
- Folder sync on mobile
- Embedding APNs provider keys in the desktop binary
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Product contract is recorded in accepted decision-4 and linked from this task
- [ ] #2 Child spikes prove notify endpoint via pairing, desktop wakeup on relevant messages, and catch-up-then-local-notification without a Leyline message server
- [ ] #3 Status copy can distinguish paired guaranteed notifications from best-effort solo mode
<!-- AC:END -->
