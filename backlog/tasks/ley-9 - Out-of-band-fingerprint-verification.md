---
id: LEY-9
title: Out-of-band fingerprint verification
status: To Do
assignee: []
created_date: '2026-10-08 02:11'
labels:
  - trust-safety
dependencies: []
type: feature
ordinal: 9000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A ticket exchange could be intercepted or impersonated in transit, and there is currently no way for two people to confirm that an `EndpointId` actually belongs to who they think it does, short of trusting the channel implicitly.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Two peers can compare a human-readable, "safety number"-style fingerprint derived from their endpoint identities
- [ ] #2 The comparison can be done out-of-band, not solely over the channel being verified
- [ ] #3 Documentation explains what a mismatch means and what action a user should take
<!-- AC:END -->
