---
id: LEY-24.1
title: Classify channel kind and resolve DM and point-to-point tab names live
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 05:45'
updated_date: '2026-10-12 06:03'
labels:
  - ux
  - channels
dependencies: []
parent_task_id: LEY-24
type: feature
ordinal: 28000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
`/msg` and `/dm` name a DM or point-to-point channel from the pet name or hex-prefix of a peer at the moment the channel is created (`AppState::unique_dm_channel_name`, `direct::channel_name`), but nothing re-evaluates that name afterward. Running `/alias` on a peer updates `AppState::display_name` everywhere it is already used (the peers sidebar, `/who`), but not the tab label of the channel itself, so a DM or point-to-point chat started before a peer name was known can stay a bare hex-prefix forever. Fixing the display is only half of it: today the only way to tell whether a given channel is an ordinary group channel, a gossip-based DM (`dm_channels`), or a point-to-point chat (`direct_channels`) is to scan those two maps by hand or parse the `dm-`/`p2p-` name prefix -- there is no single place that answers what kind a channel is and who it is with. This is the foundation the UI work in this epic (the nav panel, kind badges, quick-switcher) builds on.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 The displayed name of a DM or point-to-point channel reflects the current pet name or nickname resolution for its peer (`AppState::display_name`), rather than a name frozen at channel-creation time, including updating immediately after running `/alias` on that peer
- [x] #2 A single `AppState` query classifies any channel by kind (group, gossip DM, or point-to-point) and, for the latter two, returns the associated peer id
- [x] #3 Group channel names are unaffected by this change and continue to show their stored name rather than a resolved one
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add a ChannelKind enum (Group, GossipDm{peer}, Direct{peer}) and a single AppState::channel_kind(&str) query that classifies a channel by reverse-looking-up dm_channels/direct_channels. direct_peer_for (used by main.rs for send routing) delegates to it so there is one source of truth. (AC2)
2. Add AppState::channel_label(&str): a group channel's label is its stored name; a DM or point-to-point channel's label is resolved on every call from the peer via display_name, with a dm-/p2p- text prefix kept as an interim kind marker so a /msg DM and a /dm chat with the same person stay distinguishable until LEY-24.2 introduces kind glyphs. (AC1, AC3)
3. Show the label wherever a channel is displayed to the user: the tab bar and the three pane titles in ui.rs, /who headings, the 'left #...' notice, and main.rs's 'invite for #...' line. Channel.name stays the identity key everywhere else (storage filenames, registries, gossip map, send routing) and is never changed.
4. Finding: for the RECIPIENT of a /msg invite, the stored name (chosen by the initiator via unique_dm_channel_name) describes the recipient themselves, not the inviter, so the old tab name was wrong for them, not merely stale. Live resolution fixes that; covered by a test.
5. Consequence: displayed label can now differ from the raw name, so /leave <name> must accept what is on screen. run_leave resolves a typed name by exact raw name first, then by label, and refuses an ambiguous label with guidance instead of silently picking one.
6. Consequence: classification reads dm_channels/direct_channels, and dm_channels was never cleaned on /leave (stale entry could misclassify a later same-named group channel). remove_channel now clears both associations (replacing forget_direct_channel and its separate call in main.rs) after computing the label for its 'left' notice.
7. Tests in app.rs (ui.rs has no render tests): classification of each kind, live label after /alias and /nick, recipient-side DmJoined, dm vs p2p distinct labels, group name unaffected incl. a group literally named dm-x, /leave by label/raw/ambiguous/unknown, remove_channel association cleanup and 'left' notice label, /who heading. Update README and concept.md. Run cargo build/test/clippy/fmt.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented. ChannelKind (Group / GossipDm{peer} / Direct{peer}) + AppState::channel_kind (single reverse-lookup over dm_channels/direct_channels; direct_peer_for now delegates to it) + AppState::channel_label (group = stored name; DM/p2p = dm-/p2p- + display_name(peer), resolved on every call). ui.rs tab bar and pane titles, /who headings, the 'left #...' notice and main.rs's 'invite for #...' line now show the label; Channel.name remains the identity key everywhere else.

Decisions: (1) kept a dm-/p2p- text prefix as an interim kind marker so a /msg DM and a /dm chat with the same person do not render identically before LEY-24.2 adds glyphs; 24.2 should replace the prefix with a glyph driven by channel_kind. (2) /leave <name> resolves the typed name by exact raw name first, then by label, and refuses an ambiguous label with guidance, because what is on screen no longer always equals the raw name. (3) remove_channel now clears dm_channels and direct_channels entries (replacing forget_direct_channel and its separate main.rs call), after computing the label for its 'left' notice; previously dm_channels was never cleaned on /leave so a stale entry could misclassify a later group channel reusing the name.

Finding: for the RECIPIENT of a /msg invite the stored name is chosen by the initiator from their label for the recipient, so the old tab name described the recipient themselves, not the person they were talking to. Live resolution fixes that (test: a_dm_invite_recipients_tab_is_labeled_by_the_inviter).

Known limitation: a user-created group channel literally named like a DM label (e.g. /join dm-Alice while a DM labels as dm-Alice) yields two identically-labeled tabs until 24.2 glyphs distinguish them; /leave handles it deterministically (exact name wins) and /who, send routing and storage are unaffected because they key on the raw name.

Validation: cargo test 493 passed (+21 net: classification, live label after real /alias and /nick flows, recipient-side DmJoined, dm-vs-p2p labels, group unaffected incl. a group named dm-x, /leave by label/raw/exact-precedence/ambiguous, remove_channel association cleanup, 'left' notice and /who label, plus 4 ui.rs TestBackend render tests asserting the drawn tab bar and pane title). Mutation check: reverting the tab bar and pane title to raw channel.name made exactly the two relevant render tests fail, then restored. cargo clippy --all-targets clean. cargo fmt --check clean except one pre-existing, unrelated line in dm_registry.rs left untouched. README and concept.md updated.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added channel-kind classification and live-resolved labels for DM and point-to-point channels. New ChannelKind enum and AppState::channel_kind give one query that classifies any channel as group, /msg DM, or /dm point-to-point chat (with the peer for the latter two); AppState::channel_label shows a group channel by its stored name and a DM or point-to-point chat as dm-/p2p- plus display_name(peer), resolved on every draw, so it follows /alias and /nick immediately instead of freezing at creation time. The tab bar, pane titles, /who headings, the 'left #...' notice and the invite line show the label while Channel.name stays the unchanged identity key for storage, registries and routing. Because the label can now differ from the raw name, /leave accepts either (exact name first, ambiguous labels refused), and remove_channel now clears DM/point-to-point associations so a reused name cannot be misclassified. Also fixes the recipient side of /msg invites, whose stored tab name described themselves rather than the inviter. Verified with cargo test (493 passed): classification tests per kind, label tests driven through the real /alias and /nick flows, and ui.rs TestBackend render tests asserting the drawn tab bar and pane title (a mutation check confirmed these fail when the renderer uses the raw name). cargo clippy --all-targets is clean; cargo fmt --check is clean except one pre-existing unrelated line in dm_registry.rs. README and concept.md document the behavior.
<!-- SECTION:FINAL_SUMMARY:END -->
