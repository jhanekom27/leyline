---
id: LEY-20
title: 'Multi-device identity: link devices to one user'
status: To Do
assignee: []
created_date: '2026-10-10 13:40'
labels:
  - identity
dependencies: []
type: feature
ordinal: 20000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Using leyline from a second device creates a second, totally unrelated identity: a fresh `EndpointId` with no connection to any other device. Every already-joined channel then needs a fresh invite, and peers see two strangers instead of one person. This epic tracks linking multiple devices to one persistent user identity, so a person is represented consistently to peers across every device they use, new devices inherit channel membership without being re-invited, and file sharing keeps working across the devices owned by one person. Related existing gaps: LEY-9 (out-of-band fingerprint verification) and LEY-11 (channel-secret rotation as the remove-a-member story) both touch the same trust model this work extends.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 The devices owned by one person are represented as one identity to peers (display name, alias, presence, and /who), not as unrelated strangers
- [ ] #2 Joining a channel on one already-paired device does not require a separate invite on another device owned by the same person
- [ ] #3 A file sent with /send from one device remains fetchable with /save from another paired device owned by the same person
<!-- AC:END -->
