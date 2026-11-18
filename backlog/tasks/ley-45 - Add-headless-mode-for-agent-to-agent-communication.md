---
id: LEY-45
title: Add headless mode for agent-to-agent communication
status: To Do
assignee: []
created_date: '2026-11-18 06:39'
labels:
  - cli
  - agents
  - networking
dependencies: []
priority: medium
type: feature
ordinal: 93000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Enable leyline to run without a TUI so agents (or other automated peers) can join channels and exchange messages programmatically.

## Why
Leyline today is an interactive terminal chat UI. A headless mode would let agents use the same peer-to-peer gossip/DM fabric for agent-to-agent (or agent-to-human) communication without driving ratatui or a human keyboard.

## Outcome
Operators can start leyline in a non-interactive mode, connect with the existing ticket/pairing model, and send/receive channel or DM traffic over a machine-friendly interface (for example stdin/stdout, a simple CLI subcommand surface, or an equivalent local API). The interactive TUI path remains the default and is unchanged for human users.

## Notes
Scope is enabling the communication path for agents, not building a full multi-agent orchestration product. Prefer reusing existing networking and ticket flows over inventing a parallel stack.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Leyline can be started in a documented headless/non-TUI mode that does not require a terminal UI or interactive keyboard loop
- [ ] #2 A headless peer can join or create a channel (or DM) using the existing ticket/pairing model
- [ ] #3 A headless peer can send and receive chat messages in a machine-consumable form suitable for agent tooling
- [ ] #4 Headless mode coexists with interactive TUI peers on the same channel without breaking human chat
- [ ] #5 README or /help documents how to run headless mode and the expected I/O contract
- [ ] #6 Focused tests cover the headless entry path and message send/receive without requiring a TUI
<!-- AC:END -->
