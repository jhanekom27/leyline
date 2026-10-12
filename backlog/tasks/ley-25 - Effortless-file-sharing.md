---
id: LEY-25
title: Effortless file sharing
status: To Do
assignee: []
created_date: '2026-10-12 08:28'
labels:
  - files
  - ux
dependencies: []
type: feature
ordinal: 32000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Sharing a file takes too many manual steps: the sender has to type an exact path into `/send`, the receiver has to copy a hash prefix into `/save`, `Ctrl+V` broadcasts a file with no preview or undo, every channel saves into one flat Downloads folder, and a file cannot be fetched while its original sender is offline. This umbrella groups the work to make sending and receiving files frictionless without changing the wire format: attachments stay a `ChatMessage` attachment backed by iroh-blobs.
<!-- SECTION:DESCRIPTION:END -->
