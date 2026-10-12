---
id: LEY-24.2
title: Replace the top tab bar with a left-side Channels and People navigation panel
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 05:45'
updated_date: '2026-10-12 06:40'
labels:
  - ux
  - channels
dependencies:
  - LEY-24.1
parent_task_id: LEY-24
type: feature
ordinal: 29000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
The tab bar (`ui::render_header`) is a single horizontal row with no overflow or scroll handling -- once there are more channels and DMs or point-to-point chats than fit the terminal width, tabs become unreadable, and there is no visual grouping between ordinary group channels and 1:1 conversations beyond the name prefix. This replaces it with a scrollable left-side panel, split into a group-channels section and a people section (using the kind classification from the channel-kind task), with each row showing a kind glyph and the resolved display name. `Tab` and `Shift+Tab` keep moving through the combined list exactly as they do today, with no new modal focus state, so this is a rendering and layout change rather than a change to the interaction model.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Channels are listed in a left-side panel grouped into two sections -- group channels, and DMs/point-to-point chats (for example labeled CHANNELS and PEOPLE) -- instead of the current single-row top tab bar
- [x] #2 Each entry in the people section shows a glyph distinguishing a gossip-based DM from a point-to-point chat
- [x] #3 The panel scrolls when the combined list is taller than the available space, instead of truncating or overflowing
- [x] #4 `Tab` and `Shift+Tab` continue to switch the active channel, with no new focus mode, stepping through the channels in the order the panel draws them (group channels, then conversations, each in join order) -- the same order as before whenever only group channels exist
- [x] #5 The active channel is visually highlighted in its row, mirroring the current active-tab styling
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Navigation order (app.rs): add AppState::nav_sections (group channels, then conversations, each in join order) and nav_order; switch_channel (Tab/Shift+Tab) walks nav_order so the highlight moves through the panel top to bottom and wraps. Identical to join order whenever every channel is a group channel.
2. Kind glyphs (app.rs): ChannelKind::glyph (# group, @ /msg DM, lightning for /dm point-to-point). channel_label becomes the bare resolved name (drops 24.1's interim dm-/p2p- text prefix, now redundant next to a glyph); add channel_heading = glyph + label, used by pane titles and system notices (/who, left, invite) so kind is shown consistently everywhere, not just in the panel.
3. /leave resolution: raw name first, else the bare label or the heading (so general, #general, Alice and @Alice all work; ambiguous labels still refused with guidance).
4. Layout (ui.rs): replace the 2-row header with a 1-row identity line; add a fixed-width (24 col) bordered left panel titled 'leyline' ahead of messages and peers; CHANNELS and PEOPLE section headers always shown (stable layout, no jump when the first DM appears); each entry is a glyph column padded to 2 display cells (the lightning glyph is double-width) plus the label truncated with an ellipsis; right-aligned * for unread (never on the active row); active row Black-on-Cyan bold across the full row width, mirroring the old active tab.
5. Scrolling: stateless viewport follows the active row, centered where there is room (pure nav_scroll_offset), reusing render_scroll_indicator for a thumb when the list overflows. No new focus state or keys.
6. Tests: app.rs (sections and order, Tab/Shift+Tab in an interleaved case, glyphs/headings, /leave forms, updated 24.1 label expectations); ui.rs TestBackend (sections and order, glyph per kind and label column alignment, alias not raw name, highlight, unread marker, scrolling at small heights, identity row replaces the tab bar) plus pure nav_scroll_offset tests; mutation-check the key ones.
7. Docs: README (layout, keys, command rows) and concept.md.
8. Interpretation call, flagged: Tab/Shift+Tab follow the panel's drawn order. AC #4 says 'same relative order as before', but with interleaved joins (e.g. general, then /msg Alice, then project-x) join order would make the highlight zig-zag between sections, so the traversal is grouped to match what is on screen; identical to before when only group channels exist. AC #4 wording is clarified to say exactly that.
9. Known tradeoff, not addressed here: the panel costs 24 columns, so at ~80 columns the message text column gets narrow. Auto-collapsing the right peers sidebar on narrow terminals is a candidate follow-up.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as planned. app.rs: ChannelKind::glyph (# group, @ /msg DM, lightning bolt for /dm), NavSections/nav_sections/nav_order, a bare channel_label plus channel_heading (glyph + label, used by pane titles, /who, the left notice and the invite line), switch_channel stepping through nav_order, and /leave resolving raw name, then label, then heading. ui.rs: 1-row identity header, 24-col left panel (render_nav) with always-shown CHANNELS/PEOPLE headers, glyph column padded by measured width (the bolt is double-width), ellipsis truncation clear of a right-aligned yellow * unread marker (never on the active row), black-on-cyan full-row highlight for the active channel, and a stateless centered scroll window (nav_scroll_offset) reusing render_scroll_indicator.

Decision: Tab/Shift+Tab follow the panel drawn order (group channels, then conversations) instead of strict join order, because interleaved joins would otherwise make the highlight zig-zag between sections. Identical to the old order when only group channels exist. AC #4 wording was clarified to say exactly that (plan item 8). Easy to revert to join order if preferred: switch_channel would just step through 0..channels.len().

Decision: channel_label is now the bare resolved name; the interim dm-/p2p- text prefix from LEY-24.1 is dropped because the glyph carries the kind. A /msg DM and a /dm chat with the same person therefore share a label and are told apart by their heading (@Alice vs lightning-bolt Alice); /leave accepts either form and still refuses an ambiguous bare label. The 24.1 tests that asserted prefixed labels were updated.

Tradeoffs and follow-ups (not actioned, need approval): (a) the 24-col panel narrows the message column on ~80-col terminals (about 13 text chars); auto-collapsing the right peers sidebar on narrow terminals is a candidate follow-up. (b) The point-to-point glyph is a single value in ChannelKind::glyph so it can be swapped if a terminal renders the bolt badly. (c) COMMAND_HINTS in ui.rs still lacks /dm (pre-existing, out of scope). (d) Label truncation counts chars rather than display cells, consistent with the existing truncate_chars/fit_name; only the glyph column is measured by display width.

Validation: cargo test 518 passed (493 baseline + 25 new); cargo clippy --all-targets clean; cargo fmt --check flags only the pre-existing unrelated hunk at src/dm_registry.rs:181 (left untouched). Mutation checks, each caught by the intended tests and then restored: join-order Tab, ungrouped sections, raw-name labels, /leave without heading match, heading without glyph, switch_channel indexing channels directly, unpadded glyph column, no active highlight, no scrolling, label budget ignoring the marker column, 2-row header. Also eyeballed a TestBackend dump of the full screen and of a 12-row scrolled panel. Docs updated: README (layout paragraph, Tab row, /msg, /dm and /leave rows) and concept.md (identity vs label vs heading, panel and Tab ordering).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Replaced the top tab bar with a scrollable left-hand navigation panel split into CHANNELS (group channels) and PEOPLE (/msg DMs and /dm chats), each entry led by a kind glyph (# / @ / lightning bolt) and named by its live label, with the active channel highlighted, a right-aligned unread marker, and a stateless scroll window that keeps the active row in view. Tab/Shift+Tab walk the panel in drawn order. Channel headings (glyph + label) now name channels in pane titles, /who, left and invite notices, and /leave accepts a raw name, label or heading. Verified with 518 passing tests (25 new, including TestBackend render tests for sections, glyph/label column alignment, highlight, unread marker, truncation and scrolling, plus an exhaustive pure scroll-offset property test), clean clippy, and 11 mutation checks each caught by the intended tests. README and concept.md updated.
<!-- SECTION:FINAL_SUMMARY:END -->
