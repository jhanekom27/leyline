---
id: LEY-25.3
title: In-TUI file picker for /send
status: Done
assignee:
  - '@Jurgen'
created_date: '2026-10-12 08:28'
updated_date: '2026-10-12 15:41'
labels:
  - files
  - ux
dependencies:
  - LEY-25.1
parent_task_id: LEY-25
type: feature
ordinal: 36000
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
`/send` needs an exact path typed or pasted, which is the biggest friction when sharing from a terminal. A keyboard-driven file browser inside the TUI, in the style of the `Ctrl+P` jump-to overlay, removes that without a GUI dialog, so it also works over SSH and on Linux. Path tab-completion in the message box is related but belongs with LEY-15.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Bare `/send` with no argument opens a modal file browser; `/send <path>` behaves as before
- [x] #2 Up and Down move the highlight, Enter opens a folder or chooses a file, and Backspace goes up a level; typing narrows the current folder by name, case-insensitively
- [x] #3 Files are listed newest first so recent downloads and screenshots are on top, with folders listed before files and hidden files left out
- [x] #4 The browser opens in the folder used last time (remembered across restarts), falling back to the Downloads folder
- [x] #5 Space marks several files; Enter stages all marked files, or the highlighted one if none are marked
- [x] #6 Esc closes the browser without staging anything; an unreadable folder shows an error and leaves the browser open
- [x] #7 Opening a very large or slow folder never freezes input
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
1. State and listing (new src/picker.rs). `FilePicker` is pure state, like `ThreadView`/`SearchResults`: the folder shown and its entries, the typed filter and the matches it leaves, the highlight, the files marked (in marking order, kept across folders), the folder read in flight, and a notice. Its `handle_key` returns a `Step` (Stay, Cancel, Open(folder), Choose(files)) so the key semantics test without an `AppState`. `read_folder(dir)` is the only I/O: it lists entries (`std::fs::metadata`, which follows symlinks, so a link to a folder is a folder), leaves out dot-files and anything that is neither a folder nor a regular file, sorts folders first (by name, ignoring case) then files newest first (by modified time, then name), and reports an unreadable folder as `can't open <folder name>: <reason>` (its own name, since the one-row notice has no room for a whole path). `start_folder(data_dir)` picks the first existing folder of: the one remembered, the OS Downloads folder, home, the current folder, the root. The remembered folder lives in its own one-line text file (`browse_dir` in the data directory), not as a new field of the postcard `settings` file: postcard is not self-describing, so a longer struct would fail to decode an existing file and silently reset the saved nickname and bell setting.
2. AppState (src/app.rs). New `picking_file: Option<FilePicker>`, modal like the Ctrl+P switcher: it owns keys and pastes (a paste filters it), `is_composing()` is false while it is open, Ctrl+C still quits. A bare `/send` opens it (`/send <path>` is unchanged) and returns `InputAction::ReadFolder { request, dir }`; request ids come from one counter on `AppState`, so a late answer to an earlier read can never land in a later picker. `apply_folder_listing` applies an answer only if it is the one the picker is waiting for; an unreadable folder keeps the old listing, shows the error and leaves the picker open. A different folder starts with a clean filter, but the folder the picker opened on keeps what was typed while it was being read. Choosing returns `InputAction::StageFiles { channel, paths, folder }` for the channel the picker was opened on (a join could change the active one meanwhile) and remembers the folder for the next open.
3. Wiring (src/main.rs). `ReadFolder` runs `read_folder` on tokio's blocking pool and reports back over its own channel, the `/search` shape, so a huge or slow folder never blocks input. `StageFiles` goes through the existing `stage_files` (each file checked exactly like a `/send`ed one) and persists the folder. The start folder is seeded once at startup (`AppState::load_browse_dir`).
4. Rendering (src/ui.rs). `render_picker` draws a fixed-size overlay over everything, like the switcher: the folder in the title (tail-truncated, with the marked count), the filter row with the cursor, folders as `name/` and files with size and modified time (a check mark on marked ones), the highlight styled like the switcher's, scrolling through `nav_scroll_offset`/`render_scroll_indicator`, placeholders for reading, empty folder and no matches, and the notice in place of the key hints (cut from the front if too long, so the reason survives). The overlay box and the filter row are shared with the switcher (`overlay_area`, `render_filter_row`).
5. Keys. Up/Down move (clamped). Enter opens a folder; on a file it stages the marked files, or the highlighted file if none are marked. Space marks or unmarks the highlighted file (a folder cannot be marked). Letters narrow the folder by name, ignoring case; Backspace deletes a typed filter character and, on an empty filter, goes up a level (highlighting the folder just left). Esc closes without staging anything.
6. Tests. Unit tests for listing (order, hidden, symlinks, unreadable) and for the state machine (filter, marks, navigation, stale answers) in picker.rs; `AppState` tests (bare `/send` opens, `/send <path>` unchanged, modal guards, staging channel, remembered folder); render tests in ui.rs mirroring the switcher's; and a scripted PTY run of the real binary: open with `/send`, filter, mark, stage, cancel, an unreadable folder, a 100,000-entry folder with input staying responsive, and the folder remembered across a restart.
7. Docs: README (`/send` and `/send <path>`, the picker's keys), the /help text and the sidebar hint, features.md, concept.md layout line and event-loop note.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Built as planned: src/picker.rs (new: FilePicker state, read_folder, start folder and the remembered folder), AppState::picking_file with InputAction::ReadFolder/StageFiles, the overlay in ui.rs (sharing overlay_area and render_filter_row with the switcher), and main.rs running folder reads on the blocking pool. /help, the sidebar hint, README, features.md and concept.md are updated. The old test that pinned a bare /send printing a usage notice is removed: AC 1 replaces that behavior.

Validation: cargo test 716 passed (80 new in picker.rs, app.rs and ui.rs, one obsolete test removed), cargo clippy --all-targets clean. The real binary under a PTY, isolated HOME, screen read with pyte: 15 of 15 scenarios pass. AC 1: a bare /send opens the picker, and /send <path> still sends the file and opens nothing. AC 2: Down moves the highlight, Enter opens a folder, Backspace goes up and highlights the folder just left, typing NEW or OLD.P narrows to new.png or old.pdf. AC 3: the listing is exactly docs/, locked/, music/, new.png, mid.txt, old.pdf, with a dot-file and a dot-folder left out. AC 4: after choosing a file from docs/ the picker reopens there after a restart of the process on the same HOME, cancelling does not change it, and with nothing remembered, or a remembered folder that is gone, it opens in Downloads. AC 5: two marked files stage together and send as two messages; with nothing marked Enter stages just the highlighted file. AC 6: Esc closes without staging and without quitting; an unreadable folder shows "can't open locked: Permission denied (os error 13)" in the bottom border, keeps the old listing and the picker open, and the hints return on the next key. AC 7: in a folder of 100,000 files (debug build) the picker opened 7 ms after Enter and a key echoed in 7 ms while it was still reading; the listing took 0.23 s, Esc closed the picker in 7 ms mid-read and the late answer was ignored, and keys echoed in 27 to 43 ms in the full list. Also checked: a paste filters the picker, and a drop typed out the way Warp does it stays plain filter text behind it; Ctrl+C quits from it.

Interpretations to confirm: (1) AC 2 says Backspace goes up a level; it first deletes a typed filter character and goes up only when the filter is empty, so a typo does not lose the folder. (2) Space marks, so the filter can hold no space (a pasted one can). (3) Enter on a folder always opens it, even with files marked; Enter on a file stages the marked files, and only the highlighted one if none are marked, as AC 5 reads -- so an unmarked highlighted file is not added to the marks. (4) Marks are kept (as full paths) when moving between folders, and a filter that matches nothing does not strand them. (5) "The folder used last time" is the folder files were last chosen from, not merely browsed, so Esc never changes it; it is kept in a one-line browse_dir file in the data directory, not in the postcard settings file, which would fail to decode an existing file and silently reset the nickname and bell setting. (6) Folders sort by name ignoring case, ahead of files; files sort by modified time. (7) Hidden means a name starting with a dot; macOS and Windows hidden flags are not consulted. (8) Keys typed while the first listing is still being read are kept as the filter; opening a different folder clears it. (9) Of "very large or slow", only very large was exercised (no network mount to hand): both use the same off-loop read.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
A bare `/send` now opens a keyboard-driven file picker instead of a usage notice, so a file can be shared without typing a path -- inside the TUI, so it works over SSH and on Linux; `/send <path>` is unchanged. picker::FilePicker is pure state (AppState::picking_file) behind a fixed-size overlay in ui.rs that shares its box and filter row with the Ctrl+P switcher. It lists folders first and then files newest first, with dot-files left out; Up/Down move, Enter opens a folder or stages the marked files (the highlighted one if none are marked), Space marks, typing narrows by name ignoring case, Backspace edits the filter and then goes up a level, Esc cancels. It opens where files were last chosen from, kept across restarts in a browse_dir file in the data directory (not the postcard settings file, which would have reset the saved nickname and bell setting), else Downloads. Folder reads run on the blocking pool and answer by request id, so a huge folder never blocks input. Verified with 716 passing tests (80 new) and clean clippy, and 15 scripted runs of the real binary under a PTY covering every acceptance criterion, including a 100,000-file folder (picker open in 7 ms, a key echoed in 7 ms while it was still reading, Esc closing it mid-read) and the remembered folder surviving a restart.
<!-- SECTION:FINAL_SUMMARY:END -->
