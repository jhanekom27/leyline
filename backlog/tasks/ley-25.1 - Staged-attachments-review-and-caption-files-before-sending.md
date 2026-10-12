---
id: LEY-25.1
title: 'Staged attachments: review and caption files before sending'
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 08:28'
updated_date: '2026-10-12 13:58'
labels:
  - files
  - ux
dependencies: []
parent_task_id: LEY-25
type: feature
ordinal: 34000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Files and images go out the instant they are chosen: `Ctrl+V` and `/paste` broadcast straight away, with no preview, no caption, and no way to cancel. With no message delete yet (LEY-2), pasting the wrong clipboard shares it with the whole channel. A staging area gives every way of picking a file (drag-and-drop, the picker, the clipboard) one place to land before sending, so those entry points can share it.

`ChatMessage` already carries both text and an attachment, so a caption needs no wire change.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Choosing a file or image with `Ctrl+V` or `/paste` stages it on the active channel instead of sending it; clipboard text still sends immediately
- [x] #2 Staged attachments are shown above the input box with their name and size, and each one can be removed before sending
- [x] #3 Pressing `Enter` sends every staged attachment and clears the stage; any typed text is sent as the caption of the first attachment
- [x] #4 Staged attachments belong to the channel they were staged in and are kept when switching channels, like input drafts
- [x] #5 Files that are missing, unreadable, or over the size limit are refused when staged, with the same messages `/send` gives today
- [x] #6 `/send <path>` still sends immediately
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Single validation boundary (src/files.rs): add `StagedAttachment`/`StagedSource` plus `stage_file`/`stage_image`, carrying todays `/send` and `/paste` refusal messages verbatim; move `MAX_FILE_SIZE` and `expand_path` (with their tests) here from net.rs so `/send` and staging cannot drift apart.
2. Staging state (src/app.rs): `Channel.staged` (per channel, like input drafts), `AppState::stage_attachment`, `InputAction::SendStaged { channel, items, caption }`. `Enter` with attachments staged sends them with the typed text as the caption of the first; a `/command` still runs as a command and leaves the stage alone; `/send <path>` is untouched. `Backspace` on an empty input unstages the newest attachment; `Esc` clears the stage (after an armed reply, before leaving a thread or quitting).
3. Sending (src/net.rs): `Net::send_staged` imports items one after another (keeps order), with the caption riding on the first item that imports; `send_file` becomes validate + `send_staged`; drop `send_clipboard_image`.
4. Wiring (src/main.rs): `ClipboardFiles`/`ClipboardImage` stage on the channel they were pasted in (clipboard text still sends at once); handle `SendStaged`.
5. UI (src/ui.rs): bordered strip above the input box (name and size per row, key hints in its title, oldest rows collapse into an overflow row so the newest stays visible); render a caption together with the attachment line (today the attachment line replaces the text, so a caption would never be shown); rename `attachment_caption` to `attachment_line` so it does not clash with the new meaning of caption.
6. Tests inline per module (state transitions, validation, strip layout via ratatui TestBackend, caption rendering); update README keybindings/commands, features.md, the concept.md layout line, and the `/help` text.
7. Verify with cargo fmt, clippy, and test. A hands-on Ctrl+V check needs a real clipboard.

8. Added during implementation: `import_staged` extracted from `Net::send_staged` and `stage_clipboard_files`/`stage_clipboard_image` extracted in main.rs so the send loop and the clipboard routing are testable; `stage_file` also opens the file, so an unreadable one is refused when staged (metadata alone succeeds on it).
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Implemented per the plan: files.rs owns StagedAttachment and the shared stage_file/stage_image validation (moved MAX_FILE_SIZE and expand_path here from net.rs); app.rs gained Channel::staged, stage_attachment, InputAction::SendStaged, and the Enter/Backspace/Esc behavior; net.rs send_staged now delegates to a testable import_staged (send_file reuses it, so /send is unchanged from the outside); main.rs stages clipboard files/images through two small helpers; ui.rs draws the staged strip above the input box and now renders a caption above the attachment line (previously the attachment line replaced the text, so a caption would never have shown).
Design choices worth reviewing: Backspace on an empty input removes the NEWEST staged attachment (LIFO) and Esc clears all, so a middle attachment can only be removed by removing the ones after it; an armed reply is left alone when staged files are sent (a file is never a reply, same as /send today).
Tests: 608 passing (cargo test), clippy clean, cargo fmt applied. Not yet exercised by hand: a real Ctrl+V of a Finder-copied file, which needs the OS clipboard.

Validation: cargo test 609 passed, cargo clippy --all-targets clean. Also added beyond the original plan: import_staged and the main.rs clipboard helpers (testability), and an open-the-file check in stage_file so an unreadable file is refused at staging.
Live checks with the real binary under an isolated HOME and the real macOS clipboard (restored to empty afterwards): /send of a file stored one attachment message while a missing path stored none and logged no error; Ctrl+V with a file on the clipboard, then a typed caption and Enter, stored exactly one message carrying both the attachment and the caption (an immediate send would have stored two); Ctrl+V with plain text on the clipboard stored a plain text message with no Enter.
Not exercised live: a clipboard image and multi-file stages (covered by unit tests). Left alone: cargo fmt on main would also reflow one unrelated test in src/dm_registry.rs.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Ctrl+V and /paste now stage a clipboard file or image on the channel it was pasted in instead of broadcasting it: a bordered strip above the input box lists each staged attachment with its name and size; Enter sends them all with the typed text as the caption of the first, Backspace on an empty input removes the newest, Esc clears them, and staged files stay with their channel across tab switches. Clipboard text and /send still send at once. One validation boundary (files::stage_file/stage_image) now serves /send and staging, and the transcript now renders a caption above the attachment line (it was never shown before). Verified with 609 passing tests, clean clippy, and live runs of the real binary against the real clipboard: Ctrl+V plus caption plus Enter stored a single message with both, clipboard text sent immediately, and /send behaved as before.
<!-- SECTION:FINAL_SUMMARY:END -->
