---
id: LEY-24
title: Improve channel and peer visibility and navigation in the UI
status: Done
assignee: []
created_date: '2026-10-12 05:45'
updated_date: '2026-10-12 07:54'
labels:
  - ux
  - channels
dependencies: []
type: feature
ordinal: 27000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
DM and point-to-point channel tabs are named from the hex-prefix of a peer at creation time and never update, so a channel can stay `dm-a1b2c3d4` or `p2p-a1b2c3d4` forever even after that peer is later `/alias`ed -- the same hex-id problem features.md already called out and fixed for in-channel peer references (petnames, nicknames via `display_name`), but never threaded through to channel or tab labels. There is also no visual distinction between a group channel, a gossip-based DM, and a point-to-point chat beyond reading the `dm-`/`p2p-` name prefix, and the single-row top tab bar has no overflow or scroll handling and no quick-jump for navigating many channels at once. This parent tracks a structural pass on channel and peer visibility and navigation: classifying channel kind, replacing the top tab bar with a scrollable left-side navigation panel split into Channels and People sections, a quick-switcher for jumping to a channel by name, and a manual rename escape hatch for group channels.
<!-- SECTION:DESCRIPTION:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Delivered as four subtasks, each with its own plan, notes and checked ACs: LEY-24.1 (d86447c) classified channels as group, /msg DM or /dm point-to-point chat (AppState::channel_kind) and made DM and point-to-point names resolve live through display_name, so they follow /alias and /nick; LEY-24.2 (bf54218) replaced the top tab bar with a scrollable left-hand panel split into CHANNELS and PEOPLE, with a kind glyph per entry (# group, @ /msg DM, a lightning bolt for /dm), an unread marker and the active channel highlighted; LEY-24.3 (6c2f2ed) added the Ctrl+P jump-to switcher; LEY-24.4 (1f43632) added /rename for a local, persisted display name on group channels.

Cross-cutting decisions made along the way: the dm-/p2p- text prefix from LEY-24.1 was dropped once the glyph carried the kind (channel_label is the bare name, channel_heading is glyph plus label and is what titles and notices use); Tab and Shift+Tab step through the panel in drawn order rather than join order; the switcher is on Ctrl+P because Ctrl+K already deletes to end of line; /rename is refused for conversations, which follow /alias instead.

Follow-ups noted but not created as tasks (they need approval): auto-collapsing the right peers sidebar on narrow terminals (the 24-column panel leaves about 13 characters of message text at 80 columns); switcher extras (most-recently-used ordering, fuzzy matching, # and @ kind filters, an on-screen hint for Ctrl+P); /join of a brand-new channel whose stored name equals another channel rename gives two identical-looking rows; COMMAND_HINTS still lacks /dm; LEY-16 (mouse support for tabs) still says tab and should now mean a row in the left-hand panel.

Follow-up resolved afterwards: LEY-16 (now titled Mouse support for the channel panel and scrolling) was reworded to refer to the left-hand navigation panel instead of tabs. Wording only, no change of scope.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Channel and people navigation now reads at a glance and scales: DM and point-to-point channels show whoever they are with and follow /alias, each channel kind has its own glyph, the top tab bar is a scrollable left-hand panel with CHANNELS and PEOPLE sections, Ctrl+P jumps to any channel or conversation by name, and /rename gives a group channel a local, persisted display name. All four subtasks are Done (LEY-24.1 to 24.4); the suite is at 576 passing tests with clean clippy, each subtask was mutation-checked, and LEY-24.4 was also exercised end to end against the real binary across restarts. README, concept.md and /help are updated.
<!-- SECTION:FINAL_SUMMARY:END -->
