---
id: decision-4
title: >-
  Mobile clients are first-class peers; paired self-devices provide optional
  notification wakeups without hosted chat infra
date: '2026-10-23'
status: accepted
---
## Context

Leyline's appeal is private peer-to-peer chat with no accounts and no central chat server to run or trust (decision-1, README, concept.md). Getting Leyline onto phones still matters: people need lock-screen notifications, and SSH/terminal-emulator access cannot provide them while the session is closed.

A hosted message relay or always-on Leyline backend would solve notifications the ordinary way, but would reintroduce infrastructure and a trust surface the project deliberately avoided. Official mobile OS push (APNs/FCM) can wake a suspended app, yet that is a platform doorbell, not a place messages must live. Leyline already models one person across devices via pairing, device certificates, and a shared user key (LEY-20); a phone should reuse that model rather than become a dumb remote for a single desktop TUI.

## Decision

1. **Phone clients are first-class Leyline peers.** A mobile app owns device identity (and the paired user key), joins channels, sends and receives over iroh, and keeps local history when it is allowed to run. It is not defined as an SSH front-end or terminal shim.

2. **Reliable notifications are an optional guarantee from paired self-devices.** When another of the user's paired devices is online, that device may act as a **notification proxy**: on relevant activity it sends a **wakeup only** (no chat body). The phone wakes, reconnects over iroh, catches up via existing backfill/live paths, then shows a **local** notification built from local state.

3. **Solo / no paired online device = best-effort.** If no notifier device is online, the phone only notices messages while it can network (foreground or OS-permitted background). The UI must surface this honestly (e.g. paired notifications active vs best-effort only).

4. **No Leyline-hosted chat or push service is required for the architecture.** v1 doorbell transport is a **user-configured webhook** (e.g. ntfy topic URL) exchanged through pairing among the user's own devices. Built-in APNs/FCM publication from a desktop peer may come later as a smoother default; it still must not carry message plaintext and still must not make friends depend on Leyline-operated infra. Public iroh relays remain the same NAT-traversal dependency they are today.

5. **Doorbell is never the message path.** Push/webhook payloads are opaque wakeups (at most enough to prioritize reconnect). Channel mute/mention filtering may run on the notifier device before wake. Friends' messages continue to move only over iroh between peers.

## Consequences

- SSH/Termux/PWA are development conveniences only, not the product path for mobile notifications.
- Mobile work implies extracting or wrapping a headless Leyline core over time (TUI becomes one frontend); spikes can prove notify/wakeup before a full mobile UI exists.
- iOS background mesh without a paired notifier will remain weaker than Android; product copy must not promise Signal-class solo-phone push from pure P2P alone.
- Pairing metadata grows to carry notify endpoints (and later push tokens if APNs/FCM ships); pairing remains a trust boundary for how your devices wake each other.
- Abuse/spam risk of shipping APNs provider credentials inside the desktop binary is deferred by starting with user-chosen webhooks.
- Related UX (LEY-13 QR tickets/pairing, LEY-4 mentions) becomes more valuable once phones exist, but is not a substitute for this decision.
