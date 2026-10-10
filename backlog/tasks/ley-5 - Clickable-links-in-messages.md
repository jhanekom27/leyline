---
id: LEY-5
title: Clickable links in messages
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-08 02:11'
updated_date: '2026-10-10 12:43'
labels:
  - messaging
dependencies: []
type: feature
ordinal: 5000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
A URL typed into chat currently renders as plain text with no way to open it without leaving the TUI or manually copying the text.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A bare URL in a rendered message is wrapped so supporting terminals make it clickable (for example via OSC 8)
- [x] #2 Terminals without hyperlink support still show the URL as readable plain text
- [x] #3 No new external dependency is required
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Add new module src/hyperlink.rs: linkify(buffer, area) runs AFTER the message List widget renders, rereading each row's plain text back out of the rendered Buffer cells (ratatui Span/Style has no hyperlink concept, and Span::styled_graphemes strips control chars, so OSC 8 can't go through the normal Line/List render path -- confirmed via ratatui-core source + official ratatui hyperlink example).
2. find_links(text) pure function detects bare http://\/https:// tokens at a word boundary, trims trailing sentence punctuation, and rejects a match that still runs to the exact end of the row (indistinguishable from a URL hard-wrapped onto the next row by markdown::chunk_word) so we never link to a truncated address.
3. apply_hyperlink patches only the first/last cell of a match with the OSC 8 open/close escape sequence plus CellDiffOption::ForcedWidth(1) (ratatui 0.30 API made for this), leaving all other cells/styling untouched. No terminal-capability detection needed -- unsupported terminals just discard the escape and show the existing plain text (satisfies AC2).
4. Wire up: in ui::render_messages, capture block.inner(area) before moving the Block into the List, then call hyperlink::linkify(frame.buffer_mut(), inner) once after rendering -- covers transcript/thread/search views uniformly since they share this one render call.
5. Add mod hyperlink; to main.rs, use crate::hyperlink; to ui.rs.
6. Tests: unit tests for find_links (scheme detection, word-boundary, punctuation trim, end-of-row rejection, multiple links), buffer-level tests for linkify's cell patching. Run cargo test, cargo clippy, cargo fmt.
No new dependency required -- uses only std::num::NonZeroU16 and existing ratatui buffer/layout APIs. Full design rationale in Warp plan edfe0129-e92b-40fc-a2ff-a498380f2b16.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented src/hyperlink.rs (linkify/find_links/apply_hyperlink) and wired it into ui::render_messages via hyperlink::linkify(frame.buffer_mut(), inner), called once after the message List renders -- covers transcript/thread/search views uniformly. Added mod hyperlink; to main.rs and use crate::hyperlink; to ui.rs. cargo build, cargo test (380 passed, incl. new hyperlink tests), cargo clippy --all-targets (clean), and cargo fmt all pass. No new dependency added.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Added src/hyperlink.rs, a self-contained module that post-processes the already-rendered terminal Buffer (ratatui's Span/Style can't carry OSC 8 escapes through the normal Line/List render path -- styled_graphemes strips control chars). find_links() detects bare http(s):// URLs at word boundaries and trims trailing sentence punctuation; linkify() patches just the first/last cell of each match with OSC 8 open/close sequences plus CellDiffOption::ForcedWidth, leaving all other cells and the existing plain text untouched. Wired into ui::render_messages (mod hyperlink; in main.rs, use crate::hyperlink; + hyperlink::linkify(frame.buffer_mut(), inner) call in ui.rs), covering the transcript, thread, and search views uniformly. Verified: cargo test (380 passed, including 9 new hyperlink:: tests covering scheme detection, word-boundary rejection, punctuation trimming, end-of-row/hard-wrap rejection, multi-link rows, and exact OSC 8 byte placement with ForcedWidth on a real Buffer); cargo clippy --all-targets clean; cargo fmt applied (scoped to touched files only); Cargo.toml/Cargo.lock diff is empty, confirming no new dependency (AC3). AC1/AC2 verified via the linkify_wraps_the_matched_url_in_an_osc_8_sequence test, which confirms the OSC 8 bytes wrap the link while every visible character -- including the link's own text -- is preserved exactly as rendered, so non-supporting terminals (which discard unrecognized OSC sequences) show unchanged plain text.
<!-- SECTION:FINAL_SUMMARY:END -->
