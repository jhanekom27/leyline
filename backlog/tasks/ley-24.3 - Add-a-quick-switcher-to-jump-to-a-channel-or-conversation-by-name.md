---
id: LEY-24.3
title: Add a quick-switcher to jump to a channel or conversation by name
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 05:45'
updated_date: '2026-10-12 07:04'
labels:
  - ux
  - channels
dependencies:
  - LEY-24.2
parent_task_id: LEY-24
type: feature
ordinal: 30000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Once the channel and people list is long (see the navigation panel task), stepping through it one at a time with `Tab` and `Shift+Tab` does not scale. This adds a filterable jump-to-conversation overlay, mirroring the existing pick-mode pattern already used for `Ctrl+R` (reply target) and `Ctrl+T` (thread view): invoke it, type to filter the combined channel and people list by name, move the highlight, and confirm to switch.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A dedicated keybinding opens an overlay listing every channel and conversation, filterable by typing
- [x] #2 Typing narrows the list to entries whose resolved name contains the typed text, case-insensitive
- [x] #3 Confirming a highlighted entry switches to it and closes the overlay; cancelling leaves the active channel unchanged
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. State (app.rs): add Switcher { query, selected } and AppState::switching: Option<Switcher>, mirroring picking_reply and picking_thread (a modal pick state checked in handle_key before normal typing).
2. Keys: Ctrl+P opens it (Ctrl+K, the usual quick-switch key, is already bound to delete-to-end-of-line, so it is left alone). While open, handle_switcher_key owns every key: typing and Backspace edit the query through one edit_switcher_query helper that also resets the highlight to the first match; Up/Down move the highlight and clamp at the ends like the other picks; Enter switches and closes (does nothing while nothing matches, and leaves state untouched if the highlighted entry is already active); Esc closes with no change; Ctrl+C still quits. A bracketed paste feeds the query instead of the hidden draft.
3. Matching: switcher_matches is the panel order (nav_order) filtered to channels whose resolved label (channel_label) contains the typed text, case-insensitive, so it matches what is on screen and the Tab order; an empty query lists everything. switcher_highlight clamps the stored position in one place so a stale position (a label or channel changing under the open overlay) can never point out of range.
4. Overlay (ui.rs): render_switcher draws last, over everything: a cleared bordered box (top-anchored, centered, fixed width, height fits the matches up to 10 rows, then it scrolls with nav_scroll_offset and a thumb via render_scroll_indicator), a prompt row with the query and the terminal cursor, then one row per match built by nav_entry (generalised to take a highlighted flag) so the glyph column, label truncation and unread marker match the panel, with the highlighted row styled like the active channel. No matches shows a dim placeholder. Key hints sit on the bottom border.
5. Tests: app.rs (open, filter narrowing and case-insensitivity, label not raw name, Backspace widens, Up/Down clamp, typing resets the highlight, Enter switches and clears unread and an armed reply, Enter on the active channel is a no-op, Enter with no matches stays open, Esc cancels without quitting, Ctrl+C quits, keys never leak into the draft, paste feeds the query); ui.rs TestBackend (closed shows nothing, lists every channel with glyphs, filtering, highlight follows Down, scrolling past 10 rows, no-matches row, cursor after the query); mutation-check the key ones.
6. Docs: README keybinding row, the /help key list, concept.md.
7. Decisions to flag: Ctrl+P as the key; substring match on the label only (not fuzzy, not the kind glyph); panel order rather than most-recently-used. Not doing here: MRU ordering, fuzzy ranking, kind-prefix filters (# and @), an on-screen hint for the key; each can follow if wanted.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented as planned. app.rs: Switcher { query, selected } and AppState::switching (a modal state like picking_reply/picking_thread, dispatched from handle_key before normal typing); Ctrl+P opens it. handle_switcher_key owns every key while open: typing and Backspace go through one edit_switcher_query helper (which also resets the highlight to the first match), Up/Down move and clamp like the other picks, Enter switches and closes, Esc closes, Ctrl+C still quits, and a bracketed paste filters the query instead of landing in the hidden draft. switcher_matches is nav_order filtered by label substring; switcher_highlight clamps the stored position in one place. ui.rs: render_switcher draws last (over everything, so its cursor wins): a cleared, bordered, top-anchored 44-col box with a prompt row and the query, then rows built by nav_entry (now taking a highlighted flag) so the glyph column, truncation and unread marker match the panel; it scrolls with nav_scroll_offset and a thumb past 10 matches; a no-matches placeholder; key hints on the bottom border.

Decisions to flag: (a) Ctrl+P, because Ctrl+K (the usual quick-switch key) is already delete-to-end-of-line in the message box; one match arm to change. (b) Matching is a case-insensitive substring of the label only (what is on screen, never the stored name), not fuzzy and not the kind glyph. (c) Entries are in panel order (the same order Tab walks), not most-recently-used. (d) Enter on the channel you are already on just closes the overlay and leaves an armed reply alone; Enter with no matches keeps it open. (e) A paste goes to the query with line breaks and other control characters dropped.

Not done, needs approval: MRU ordering, fuzzy ranking, kind-prefix filters (# and @ to filter by kind), and an on-screen hint for the key (for example on the navigation panel border). The key is documented in the README, the /help key list and concept.md.

Validation: cargo test 549 passed (518 baseline + 31 new: 19 app.rs behavior tests, 12 ui.rs TestBackend and geometry tests); cargo clippy --all-targets clean; cargo fmt --check flags only the pre-existing unrelated hunk at src/dm_registry.rs:181 (left untouched). Mutation checks, each caught by the intended tests and then restored: case-sensitive matching, matching the stored name instead of the label, Enter not closing, choosing the active channel clearing an armed reply, Esc confirming, query edits not resetting the highlight, unclamped stored highlight, Up/Down wrapping, a paste also reaching the draft, Ctrl+C not quitting, control chords typing into the query, no highlighted row, no Clear under the box, no cursor in the query row, overlay drawn before the input box, no scrolling, uncapped box height, no scroll thumb. Also eyeballed a TestBackend dump of the overlay in three states (full list with highlight, filtered, no matches).
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added a Ctrl+P jump-to overlay: a cleared, bordered box over the top of the screen with a prompt row and every channel and conversation below it in panel order, in the same glyph, label and unread-marker format as the navigation panel. Typing (or pasting) narrows the list to labels containing the text, ignoring case; Up/Down move the highlight (clamped, scrolling with a thumb past 10 matches); Enter switches to the highlighted entry and closes the overlay, Esc cancels leaving the active channel untouched. State is only the query and a highlight position, with the matches re-derived from the live channel list so a rename or a join while it is open cannot leave it stale. Verified with 549 passing tests (31 new, including TestBackend render tests and behavior tests), clean clippy, and 18 mutation checks each caught by the intended tests. README, the /help key list and concept.md updated.
<!-- SECTION:FINAL_SUMMARY:END -->
