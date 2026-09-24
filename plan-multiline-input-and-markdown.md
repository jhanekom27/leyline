# Multi-line input and Markdown rendering

> Parked implementation plan, drafted but not yet built -- kept here for
> reference until there's appetite to tackle it. See [features.md](./features.md)
> for the lighter-weight wishlist this graduated from.

Assess the effort for two chat-box features and implement them: (1) letting the input box accept multi-line, spaced-out messages instead of submitting on every `Enter`, and (2) rendering markdown syntax (headers, code blocks, bold/italic, inline code) typed into the input as styled output in the transcript.

## Current state (researched, not assumed)
- Input is a single logical line: `AppState.input: String` + `AppState.cursor: usize` (character index), edited by simple char-based methods (`src/app.rs:229-230`, `:1255-1339`). `Enter` always calls `submit_input()` (`src/app.rs:475`, `:726-740`); there is no way today to insert a literal newline.
- Bracketed-paste (`AppState::paste_text`, `src/app.rs:1269-1286`) deliberately flattens `\n`/`\r\n` to spaces "since the input box is a single line by construction" -- the one place today that actively defeats multi-line content.
- The input box renders as one `Paragraph` line with horizontal-only scroll-to-cursor (`ui::render_input`, `src/ui.rs:708-741`), inside a fixed 3-row area (`ui::render`, `src/ui.rs:63-74`: `Constraint::Length(3)`).
- The transcript side already copes with embedded newlines: `push_chat_rows` (`src/ui.rs:400-460`) wraps message text with `textwrap::wrap`, and that crate's `wrap()` splits the input on `\n` *before* word-wrapping each line (confirmed in `textwrap-0.16.4/src/wrap.rs:188-191`), so a message containing literal newlines already displays as separate lines today -- this is purely an input-composition gap, not a rendering one.
- `ChatMessage.text` is a plain `String` (`src/message.rs:24`), round-tripped as-is through postcard, `storage.rs`, `backfill.rs`, and gossip, and matched with a plain case-insensitive `.contains()` in `search.rs:48-49`. None of that assumes single-line text, so multi-line support needs **no wire-format, storage, or search changes**.
- `AppState::submit_input`'s empty-check (`self.input.trim().is_empty()`, `src/app.rs:727`) already treats a message consisting only of newlines/whitespace as empty (Rust's `trim()` strips `\n` too) -- no change needed there.
- No markdown rendering exists anywhere; `push_chat_rows` only ever produces `Span::raw`/search-highlighted spans from plain wrapped text.
- No CI config, `justfile`, or `Makefile` exists -- validation is plain `cargo build`/`test`/`fmt`/`clippy`.

## Difficulty assessment
- **Multi-line input: low-to-medium.** Fully contained to `src/app.rs` and `src/ui.rs`, no new dependency, no wire/storage changes. The main work is a new "insert newline" keybinding and reworking the input box's rendering from a single scrolling line to a small, dynamically-sized, vertically-scrollable block.
- **Markdown rendering: medium.** No wire/dependency blockers either, but there's no existing width-aware *styled* text wrapper (`textwrap::wrap` only handles plain strings), and the new styling has to compose with the existing search-term highlighting instead of clobbering it. This is the bulk of the effort.

## Multi-line input
### Entering a newline without submitting
Crossterm's legacy (non-Kitty-protocol) key parsing was checked directly (`crossterm-0.29.0/src/event/sys/unix/parse.rs`): a bare `Enter` sends `\r` with no modifiers (line 92-94); `Shift+Enter` has no legacy encoding and is indistinguishable from plain `Enter` on most terminals without opting into the Kitty keyboard protocol (which leyline doesn't enable). By contrast:
- **Alt+Enter** is parsed reliably: any `ESC`-prefixed byte sequence gets `KeyModifiers::ALT` OR'd onto the parsed key (parse.rs:78-88), so `ESC`+`\r` becomes `KeyCode::Enter` + `ALT` on any terminal using the standard xterm Alt-prefixing convention.
- **Ctrl+J** is even more portable: it's the plain ASCII control byte `0x0A`, parsed unconditionally as `Ctrl+J` in raw mode (parse.rs:106-109), independent of terminal Alt/Meta configuration.

Proposal: bind both **Alt+Enter** and **Ctrl+J** to "insert a newline", added as guarded match arms in `AppState::handle_key` (`src/app.rs:449-494`) *before* the existing unguarded `KeyCode::Enter => submit_input()` arm so plain `Enter` keeps submitting. Document both in `run_help` (`src/app.rs:798-807`) and the README keybindings table.

### Input model changes (`src/app.rs`)
- Add `insert_newline` (just `self.insert_char('\n')`).
- `paste_text` (`:1269-1286`): stop flattening `\n`/`\r\n` to spaces; normalize `\r\n`/lone `\r` to `\n` and insert it literally instead. It must still never trigger a submit mid-paste (already guaranteed today since `paste_text` never calls `submit_input`).
- Redefine "line" for `move_home`/`move_end`/`delete_to_start`/`delete_to_end` (`:1247-1253`, `:1310-1320`) from "whole buffer" to "the current line" (bounded by the nearest `\n` on each side, or the buffer edges) via a small shared helper that finds the current line's start/end char indices. `Ctrl+A`/`Ctrl+E`/`Ctrl+U`/`Ctrl+K` then naturally become per-line, matching standard multi-line editor behavior.
- `move_left`/`move_right`/`delete_backward`/`delete_forward`/`delete_word_backward` need **no changes** -- they already operate on the flat char index, so they already cross a `\n` boundary the same way any other character would (e.g. `Left` at column 0 lands at the end of the previous line).

### Rendering changes (`src/ui.rs`)
- `render()` (`:63-74`) must size the input area dynamically instead of the fixed `Constraint::Length(3)`: compute the number of visual rows the current `app.input` needs at the pane's width (split on `\n`, same horizontal-scroll-per-line as today -- no word-wrapping inside the input box, to avoid the much harder problem of mapping the cursor through wrapped text), clamp it between 1 and a new `MAX_INPUT_VISIBLE_LINES` constant (proposed: 6), and add 2 for borders. `render_body`'s middle area continues to take whatever remains (`Constraint::Min(1)`), so it shrinks slightly while composing a long message.
- `render_input` (`:708-741`) is rewritten to: split `app.input` on `\n`; horizontally scroll only the line containing the cursor exactly like today's single-line logic, and clip other lines from column 0; vertically window the set of logical lines so the cursor's line is always visible once the line count exceeds the box's visible rows (same "widen/shift window to keep the selection visible" idea `render_messages` already uses for reply-picking, `:171-179`, just applied vertically here); and compute `frame.set_cursor_position` as a (row, col) pair instead of a fixed row.
- Small necessary side-fix: `reply_snippet` (`:583-589`) quotes a parent message's raw text on a single unwrapped row (reply preview and the input's reply banner). A multi-line parent's embedded `\n` would break that single-row invariant, so flatten `\n` to spaces there (mirroring what `paste_text` used to do) before truncating.

### Tests to update/add
- Replace `paste_text_flattens_embedded_newlines_to_spaces` and update `paste_text_does_not_submit_even_when_it_contains_a_slash_command` (`src/app.rs` tests) for the new literal-newline behavior.
- Add tests for: newline insertion via Alt+Enter/Ctrl+J, per-line Home/End/Ctrl+U/Ctrl+K, submitting a multi-line message, and (in `ui.rs`) a multi-line message rendering as multiple transcript rows.

## Markdown rendering
### Approach: small hand-rolled renderer, not a dependency
Two real ratatui-markdown crates were evaluated:
- **`tui-markdown`** (mature, MIT/Apache-2.0, actively maintained, ratatui-0.30-compatible via `ratatui-core`): converts markdown to a ratatui `Text`, but does **no width-aware wrapping** of its own -- integrating it would still require building the same "wrap a styled `Vec<Span>` to N columns" logic this plan needs anyway, on top of a new dependency.
- **`ratatui-markdown`**: does have built-in width-aware wrapping (`MarkdownRenderer::new(max_width)`), but is brand-new (first published 2026-05), has very low adoption (~2.3k downloads, 4 dependents), pins `ratatui ^0.29` (a semver-incompatible major/minor vs. leyline's `0.30.2`, which would make its `Line`/`Span` a distinct, incompatible type), and its crates.io license metadata (MIT OR Apache-2.0) disagrees with its actual repo (`license = "SySL-1.0"`, a non-standard "Synthetic Source License"). Rejected on maturity/licensing grounds.

Given neither dependency cleanly avoids writing a styled-wrap layer, and per the project's own preference for lean, boring, purpose-built code over general-purpose machinery, this plan adds a small new `src/markdown.rs` module instead, tailored to leyline's existing per-row rendering pipeline (colored rail + time + name prefix per row, from `push_chat_rows`).

### Scope (v1)
Headings (`#`, `##`, `###`), fenced code blocks (` ``` `), inline code (`` ` ``), bold (`**`), italic (`*` or `_`). Explicitly out of scope for now (flagged as easy follow-ups, not built): lists, blockquotes, tables, links/images, strikethrough, nested inline styles, code syntax highlighting, and any live preview while typing (typed source stays plain text in the input box; rendering only happens in the transcript, matching how the feature was described).

### Design
- `src/markdown.rs`: a line-oriented block parser (deliberately not a paragraph-joining CommonMark parser -- each source line stays its own block/wrap unit, consistent with `textwrap::wrap`'s existing per-line behavior and with multi-line input's line breaks being intentional). Produces a small `Block` enum (`Heading(level, text)`, `CodeBlock(lines)`, `Paragraph(line)`, `Blank`) by scanning lines, toggling "inside a fenced block" mode on ` ``` ` lines.
- Rendering: headings get a bold/accent styled single row (word-wrapped like today if too long); code block lines are wrapped by raw character chunks (reusing the existing char-chunk approach `wrap_chars` already uses for unbreakable system lines like invite tickets, `src/ui.rs:617-627`) rather than word-wrapped, since code shouldn't be reflowed; paragraph lines go through an inline tokenizer that turns `**bold**`/`*italic*`/`` `code` `` runs into styled "words", then greedily packs those styled words into rows up to the available width -- the same job `textwrap` does today, just style-aware. Output shape: `Vec<Vec<Span<'static>>>`, one inner vec per rendered row, matching what `push_chat_rows` already needs per row.
- Integration in `push_chat_rows` (`src/ui.rs:400-460`): only feed `message.text` through the new renderer when there's no attachment caption (`attachment_caption` output is a synthesized string, e.g. filenames, and must never be parsed as markdown); captions keep today's plain `textwrap::wrap` path unchanged.
- Search highlighting composition: today `highlighted_spans`/`find_highlights` (`src/ui.rs:491-546`) highlight a matched substring in an otherwise-plain row. With styled markdown spans, add a `highlight_spans(spans, term_lower)` that concatenates a row's span text to find match offsets (reusing `find_highlights` as-is), then re-slices the row's already-styled spans at those offsets and overlays the highlight style with `Style::patch` (confirmed available in `ratatui-core-0.1.2/src/style.rs:471`, which layers `other`'s explicit fields over `self` and leaves the rest untouched) so a search match inside bold/code text keeps its markdown styling plus the highlight. The existing single-string `highlighted_spans` becomes a thin wrapper (`highlight_spans(vec![Span::raw(text)], term_lower)`) so there's one implementation.
- `reply_snippet` (`:583-589`) intentionally keeps quoting **raw** markdown source (not re-rendered) in reply previews/banners -- re-rendering a truncated, possibly mid-syntax snippet risks visibly broken markdown, and it already needs the newline-flattening fix above.

### Tests to add
- `markdown.rs` unit tests: each block type parses correctly; inline bold/italic/code tokenizing; wrapping long lines (both paragraph and code) at a given width; blank-line handling.
- `ui.rs` tests: a message with a heading/code block/bold/italic renders the expected styled rows; a search match inside a styled span keeps both stylings (via `highlight_spans`); an attachment caption is never markdown-parsed.

### Docs
Update the README keybindings table (new newline keybinding) and `features.md`'s wishlist to reflect what's shipped, matching the project's existing convention of checking off implemented ideas there.

## Validation
`cargo test` (unit tests above plus the existing suite in `app.rs`/`ui.rs`/`message.rs`), `cargo fmt`, and `cargo clippy` -- no other lint/CI tooling exists in this repo today.
