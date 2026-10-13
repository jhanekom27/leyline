---
id: LEY-25
title: Effortless file sharing
status: To Do
assignee: []
created_date: '2026-10-12 08:28'
updated_date: '2026-10-13 04:45'
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

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Verification against this description, after LEY-25.8 (the epic has no acceptance criteria of its own, so the description's claims are what was checked).

The five pains, on the real binary: three instances (A sends, B receives, C is a third peer), isolated homes, each in a PTY read with pyte (scratch script outside the repo, /tmp/ley-25/epic.py), 35 of 35 checks pass. (1) Typing an exact path: a bare `/send` opens a picker that lists Downloads, typing narrows it, Enter stages the file, and a dropped path (a bracketed paste) is staged instead of typed into the box. (2) Copying a hash prefix: a bare `/save` fetches the newest file from another peer, and `/files` lists what was shared and saves a file with Enter. (3) Broadcast with no preview or undo: a staged file shows in a strip, Backspace takes the newest back and Esc clears the rest, nothing reaches the other peers until Enter, and the typed text becomes the caption. (4) One flat folder: a saved file lands in <Downloads>/leyline/general/ and that is the only folder made. (5) Sender offline: C saves a file, A quits, and B, who never had it, still fetches it, with the shared bytes and no `no reachable peer` complaint.

Same build, regression: the LEY-25.7 script 105 of 105, the LEY-25.8 script 37 of 37, cargo test 887 passed, cargo clippy --all-targets clean.

Wire format unchanged, as the description says: from the parent of the first LEY-25 commit (e57ef32) to HEAD no commit touches src/message.rs, src/ticket.rs, src/direct.rs or src/pairing.rs; the net.rs and backfill.rs diffs add no wire type, and the only protocol-id lines in them are in test helpers; `fetch_file` takes a list of providers, which is client-side only. An attachment is still a `ChatMessage` attachment backed by iroh-blobs.

Subtasks: LEY-25.1 to 25.6 are Done with every criterion checked; LEY-25.7 and LEY-25.8 are built with every criterion checked and wait for review of their listed interpretations. README, `/help` and the hints panel agree on /send, /save, /folder, /maxsize, /files and /paste.

Not exercised on the real binary: `Ctrl+V` and `/paste` of the real clipboard and a `/msg` DM (both would use the real clipboard); they are covered by unit tests. The one limit still open is that a file shared further back than a channel's last 500 lines cannot be listed or saved; it is tracked as LEY-27.1.
<!-- SECTION:NOTES:END -->
