---
id: LEY-25.2
title: Treat a pasted file path as a dropped file
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 08:28'
updated_date: '2026-10-12 15:04'
labels:
  - files
  - ux
dependencies:
  - LEY-25.1
parent_task_id: LEY-25
type: feature
ordinal: 35000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Dragging a file onto the terminal makes the terminal paste its path, and leyline inserts that into the message box as plain text, so the sender still has to type `/send ` in front by hand. Bracketed paste is already enabled, and `/send` already strips the quotes some drag-and-drop sources add. Terminals report only the path, never where the file was dropped, so the target is always the active channel. Not every terminal pastes it: Warp types the shell-escaped path into the pty as ordinary keys, with no bracketed-paste markers, so a drop there arrives as a burst of key presses and has to be recognized as a drop too (see the plan and notes).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 A bracketed paste made up only of existing file paths (backslash-escaped, quoted, or `file://` URIs; separated by spaces or newlines) stages those files instead of inserting text
- [x] #2 A paste that is not entirely existing paths, including ordinary text that merely mentions a path, is inserted into the message box unchanged
- [x] #3 Several dropped files are staged together; any path that is missing or unreadable is reported and skipped
- [x] #4 Dropping a file onto leyline works in Warp and in at least one other terminal on macOS or Linux (verified manually)
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. Detection (src/files.rs): `dropped_files(text) -> Option<Vec<PathBuf>>`. A paste counts as a drop only if it is nothing but existing regular files, all or nothing. It is read first as shell words (backslash escapes, single and double quotes, `file://` URIs percent-decoded; separated by ASCII whitespace only, so macOS screenshot names containing U+202F stay one path), and failing that as one raw path per line (kitty and older Alacritty paste spaces unescaped). Only absolute paths count, so a relative word like README.md, a sentence that mentions a path, a directory, or a path that does not exist leaves the whole paste as ordinary text.
2. Mode guard (src/app.rs): `AppState::is_composing()` is false while the Ctrl+P switcher, a reply or thread pick, or a pairing confirmation owns input, so a drop in those modes stays a plain paste.
3. Wiring (src/main.rs): a testable `handle_paste` for `Event::Paste` stages a drop on the active channel through the existing staging helper (renamed `stage_files`, shared with Ctrl+V), else falls through to `app.paste_text`. Files that exist but cannot be staged (unreadable, too large) are reported and skipped, as for the clipboard. Because every path must exist for the paste to count as a drop, a path that is already missing makes the whole paste text (AC 2); AC 3 covers refusals and a file vanishing between detection and staging.
4. Tests: parser unit tests for each terminal spelling (Terminal.app/iTerm2 backslashes, iTerm2 single quotes, WezTerm Posix double quotes, VTE single quotes, file URIs, newline-joined, raw), filesystem-gated tests, `handle_paste` tests, and a scripted PTY run of the real binary that sends raw bracketed pastes and inspects the persisted messages.
5. Docs: README keybindings, features.md, concept.md layout line.
6. AC 4 (Warp plus one other terminal, verified manually) needs real drags in a GUI, which I cannot perform: it stays for a hands-on check.

7. Warp types a drop instead of pasting it (found with the first real drag, see the notes): its drop handler writes the shell-escaped paths to the pty as plain bytes, so no `Event::Paste` ever arrives. New src/term_events.rs: `TermEvents` wraps crossterm's `EventStream` and, after each event it waits for, drains whatever is already queued with `futures::poll!` (it polls with the task's real waker; `now_or_never` would hand the stream a no-op one and could leave the next key press unseen until the 250 ms tick). Each run of plain typed characters that `files::dropped_files` accepts comes out as the one `Event::Paste` a bracketing terminal would have sent, so `handle_paste` stages it like any other drop; every other event passes through in order. A person types one key at a time, a drop arrives in one write, and the filesystem must confirm every path, so typing is never mistaken for a drop. The only await point is the stream's own `next`, so it stays cancel-safe inside `tokio::select!`. main.rs reads the terminal through it.

8. Tests for the new module: the pure `typed_drops_as_pastes` against Warp's exact escaping (one file, several files); typed text, a sentence naming a real file, a missing path and a directory left as key presses; order kept around other events; modified or released keys not counting as typing. Plus the scripted PTY run of the real binary, now writing Warp's exact bytes in a single write.

9. Docs: the README keybinding row, features.md and concept.md (an event loop note and the layout line) say a drop works whether the terminal pastes the path or types it out.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Validation: cargo test 630 passed (21 new), cargo clippy --all-targets clean.
Real-binary check: a PTY driver sent raw bracketed pastes to the built binary under an isolated HOME and decoded the persisted messages. Staged and sent as file attachments: a backslash-escaped path with a trailing space (Terminal.app and iTerm2 style), two single-quoted paths joined by a newline, a percent-encoded file:// URI, a raw path with a space (kitty style), and a macOS screenshot name containing U+202F; a drop followed by a typed caption became one message carrying both. Sent as plain text exactly as pasted: a sentence mentioning a real file, a path that does not exist, a directory, and a relative name (Cargo.toml) that matches a file in the working directory.
Interpretation to confirm: AC 2 says a paste that is not entirely existing paths stays text, and AC 3 says a missing path is reported and skipped. Both hold if they apply at different moments: a path already missing when pasted makes the whole paste text (so a sentence or a stale path is never swallowed), while a file that exists but cannot be staged (unreadable, over the size limit) or vanishes between detection and staging is reported and skipped, with the rest staged.
Design notes: only absolute paths count and only regular files, so a directory path pasted as text stays text; words split on ASCII whitespace only; a drop is not staged while the switcher, a pick or a pairing confirmation owns input (AppState::is_composing). Warp does not document what it pastes on a drop, so the parser accepts every spelling seen in other terminals (backslash-escaped, single or double quoted, file URIs, newline-joined, raw).
AC 4 not yet verified: it needs a real drag in Warp and in one other terminal, a GUI action I cannot perform. Left unchecked, task left In Progress.

First real drag, in Warp: the path landed in the message box as text and nothing was staged, so AC 4 failed there. Cause, read from Warp's open-source terminal code (master): `TerminalView::drag_and_drop_files` ends in `typed_characters_on_terminal`, which writes `escaped_paths_str` (each path with a backslash before every shell-special character, joined by spaces, plus one trailing space) to the pty as raw bytes. Only `Cmd+V` (`TerminalView::paste`) wraps its text in bracketed-paste markers, so a drop never reaches leyline as an `Event::Paste`. Reproduced under a PTY: the pre-fix build types the path into the box and stages nothing, while a bracketed paste of the same path stages.

Validation of the Warp fix: cargo test 637 passed (7 new, in term_events.rs), cargo clippy --all-targets and cargo fmt --check clean. PTY run of the real binary under an isolated HOME, screen read with pyte (`uv run --with pyte`), Warp's exact bytes in one write: 14 of 14 scenarios pass on the fixed build. A Warp-style drop stages the file and leaves the box empty; a drop then a typed caption becomes one message; two files stage together; a drop with text already in the box stages and keeps that text; a bracketed paste still stages. Left as text: a burst of ordinary text, a sentence naming a real file, a missing path, a real path typed by hand one key at a time, and a drop while the Ctrl+P switcher is open. Characters followed by Enter in one write keep their order. A single key echoes in about 7 ms (max 10), including right after a burst, so the stream never misses a wake-up. 60 of 60 repeated drops staged (and logged), and single-write drops of 14, 40 and 100 files (1.8, 5.3 and 13.2 KB) each arrived whole and staged. The same Warp-style scenario fails against the pre-fix build.

Limits: this leans on a drop arriving in one piece (it did at every size tried on macOS; a terminal that trickled one out would get typed text, as before), and a path pasted as text into a terminal that does not bracket pastes now stages too, the same call a bracketed paste of it already got. AC 4 stays unchecked: it needs a real drag in Warp (now expected to stage) and in one more terminal, a GUI action I cannot perform, and the Warp source above was read from master rather than the installed version.

AC 4 verified manually by Jurgen on macOS after the term_events fix: dragging a file onto leyline stages it in Warp and in Ghostty. cargo test (637 passed) re-run on the committed tree with a clean working tree.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Dragging a file onto leyline now stages it for review and a caption instead of typing its path into the message box. files::dropped_files recognizes a paste made up of nothing but existing files (backslash-escaped, quoted, file:// URIs, newline-joined or raw), all or nothing, and main.rs's handle_paste stages them on the active channel like a Ctrl+V of a copied file; AppState::is_composing keeps a drop plain text while the switcher, a pick or a pairing confirmation owns input. Warp types a drop out as plain keys instead of pasting it, so term_events::TermEvents turns keys that arrive together and spell nothing but existing files into the one paste a bracketing terminal would have sent. Verified with 637 passing tests and clean clippy, the real binary driven through a PTY with Warp's exact bytes (14 scenarios, including repeated drops, ordinary text staying text and no input lag), and by hand: a real drag stages the file in Warp and in Ghostty.
<!-- SECTION:FINAL_SUMMARY:END -->
