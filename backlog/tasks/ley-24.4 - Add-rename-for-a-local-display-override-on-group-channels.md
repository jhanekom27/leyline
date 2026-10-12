---
id: LEY-24.4
title: Add /rename for a local display override on group channels
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 05:46'
updated_date: '2026-10-12 07:29'
labels:
  - ux
  - channels
dependencies:
  - LEY-24.1
parent_task_id: LEY-24
type: feature
ordinal: 31000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Group channels only ever show the name they were created or joined under, with no way to shorten or relabel one locally when it is inconvenient, for example a long project name, or two similarly named channels from different friends. `/alias` already covers this for people -- renaming here is specific to group channels, since a DM or point-to-point conversation displayed name already resolves from the peer alias (see the channel-kind task), and having two independent overrides for the same 1:1 conversation would be confusing. This is a local-only override that is never sent over the wire, mirroring the pet-name pattern in contacts.rs.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 `/rename <name>` sets a local display override for the active group channel, shown in the navigation panel and anywhere else the channel name is displayed
- [x] #2 The override persists across restarts
- [x] #3 Running `/rename` on a DM or point-to-point conversation is refused with a message pointing to `/alias` instead
- [x] #4 Leaving a renamed channel forgets its override, so a later rejoin under the same name does not inherit a stale display name
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Store (new channel_renames.rs): ChannelRenames mirrors contacts.rs (postcard file, whole-file rewrite on change, a corrupt file tolerated as empty): load, all() -> HashMap<String, String> keyed by the stored channel name, set, forget (no write when there is nothing to forget).
2. State (app.rs): AppState::renames, seeded by load_renames. channel_label shows a group channel as its override (DMs and point-to-point chats ignore it, they follow /alias), so the navigation panel, pane titles, /who, the left and invite notices, /leave matching and the Ctrl+P switcher all pick it up with no further change. remove_channel drops the entry.
3. Command: /rename <name> on the active channel updates renames immediately and returns InputAction::Rename { channel, name: Some } for main.rs to persist. Refusals, all checked up front: a DM or point-to-point chat, with a message pointing at /alias; and a name that could be mistaken for another channel, meaning another channel stored name (which /leave matches before anything displayed, so a hidden stored name would otherwise shadow the shown one) or what another group channel already shows (two identical #name rows could not be told apart). A bare /rename restores the stored name (InputAction::Rename with None), mirroring how a bare /search clears a search, and shows usage when there is nothing to restore.
4. Wiring (main.rs): load the store, seed AppState, persist InputAction::Rename, forget on InputAction::Leave (so a later rejoin under the same name starts clean), and add the file to the list a /pair redeem wipes.
5. Discoverability and docs: /rename in the /help list and the sidebar command hints, a README commands row, concept.md.
6. Tests: the store (mirrors contacts.rs); app.rs (set, label wherever it is shown, replace, restore, usage, DM and point-to-point refused with a pointer to /alias, both clash checks, leave forgets across a rejoin, seeding, DM labels unaffected, /leave and the switcher by the new name); a ui.rs render test; a main.rs test that the pairing wipe list includes the file; mutation-check the key ones; and, if the app starts offline, a real-binary smoke test under a throwaway HOME for the restart and leave paths the unit tests cannot reach.
7. Decisions to flag: a bare /rename restoring the stored name goes beyond the four ACs, but without it there is no way back short of leaving the channel; names clashing with another channel are refused; the override is local only, so an invite ticket and the net-level messages that name a channel (a clashing-ticket join message, join failures, the pre-TUI ticket listing) still use the stored name; joining a brand-new channel whose stored name equals another channel rename is not intercepted.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as planned. New channel_renames.rs (ChannelRenames, mirroring contacts.rs: postcard file, whole-file rewrite, corrupt file tolerated as empty; load, all, set, forget). app.rs: AppState::renames seeded by load_renames; channel_label shows a group channel as its rename (conversations ignore it and keep following /alias), so the navigation panel, pane titles, /who, the left and invite notices, /leave matching and the Ctrl+P switcher pick it up unchanged; remove_channel drops it; /rename [name] (run_rename) returns InputAction::Rename { channel, name: Option } for main.rs to persist. main.rs: loads the store, seeds AppState, persists Rename, forgets on Leave, and adds channel_renames to the files a /pair redeem wipes. /rename is also in the /help list and the sidebar command hints; README and concept.md updated.

Decisions to flag: (a) A bare /rename restores the stored name. That goes beyond the four ACs, but without it there is no way back short of leaving the channel; it mirrors how a bare /search clears a search, and shows usage when there is nothing to restore. (b) A name is refused when it could be mistaken for another channel: another channel stored name (which /leave matches before anything displayed, so a stored name hidden behind a rename would otherwise shadow the one on screen), or what another group channel already shows. A group channel may share a name with a conversation (the # and @ glyphs set them apart), and a channel may be renamed to its own stored name. (c) The rename is local only, so invite tickets and the messages that name a channel by its stored name (the clashing-ticket join message, join failures, the ticket listing printed before the TUI starts) are unchanged. (d) Not intercepted: /join of a brand-new channel whose stored name equals another channel rename gives two identical-looking rows; /rename one of them to tell them apart.

Validation: cargo test 576 passed (549 baseline + 27 new: 9 store, 16 app.rs, 1 ui.rs, 1 main.rs); cargo clippy --all-targets clean; cargo fmt --check flags only the pre-existing unrelated hunk at src/dm_registry.rs:181 (left untouched). 13 mutation checks, each caught by the intended tests and then restored: group label ignoring the rename, leave keeping the rename in memory, a conversation honouring a stored rename, /rename allowed on a DM or point-to-point chat, no clash check, clash check ignoring another channel stored name, clash check ignoring another group channel label, bare /rename keeping the name, /rename returning no action to persist, /rename not recording the name, forget not written to disk, forget of nothing rewriting the file, and the pairing wipe list missing the file.

End-to-end smoke test of the real binary (pseudo-terminal under a throwaway HOME, output rendered through a terminal emulator), three runs: /rename shows the new name in the panel and the pane title; a bare /rename restores; a name another channel shows is refused; the name survives a restart (AC2); /leave by the displayed name followed by /join under the stored name shows the stored name, the persisted entry is gone, and a further restart is still clean (AC4); no rename warnings in leyline.log. AC3 (refusing a DM or point-to-point chat) is covered by the unit tests only, since exercising it end to end needs a peer.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added /rename: /rename <name> gives the active group channel a local display name that shows everywhere the channel is named (navigation panel, pane title, /who, left and invite notices, /leave and the Ctrl+P switcher) and persists across restarts in a new channel_renames store modelled on contacts.rs. It is never sent to peers. A DM or point-to-point chat is refused with a pointer to /alias, a bare /rename restores the stored name, a name that could be mistaken for another channel is refused, and leaving a channel forgets its name so a later rejoin under the same stored name starts clean. Verified with 576 passing tests (27 new), clean clippy, 13 mutation checks each caught by the intended tests, and an end-to-end run of the real binary across restarts. README, concept.md, /help and the command hints updated.
<!-- SECTION:FINAL_SUMMARY:END -->
