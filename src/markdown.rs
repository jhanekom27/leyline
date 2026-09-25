//! Minimal, line-oriented markdown rendering for chat messages.
//!
//! Deliberately not a CommonMark parser: each source line stays its own
//! block/wrap unit (matching how `textwrap::wrap` already split a
//! message's embedded newlines before this module replaced it in
//! `ui::push_chat_rows`, and how multi-line input's line breaks are
//! intentional -- see `app::AppState::insert_newline`), rather than
//! joining consecutive non-blank lines into one reflowed paragraph.
//!
//! Supports headings (`#`/`##`/`###`), fenced code blocks (three
//! backticks), inline code (single backtick), bold (`**`), italic
//! (`*` or `_`), unordered lists (`-`/`*`/`+`), ordered lists (`N.`),
//! and blockquotes (`>`, nested via repeated `>`). Deliberately out of
//! scope for now: tables, links/images, strikethrough, nested inline
//! styles, task lists, and code syntax highlighting -- see
//! plan-multiline-input-and-markdown.md.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

/// A block-level element scanned from a message's source lines.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Block {
    /// `#`/`##`/`###` followed by a space; level is 1-3.
    Heading(u8, String),
    /// Lines between a pair of fence lines (or from an unterminated
    /// fence to the end of the text).
    Code(Vec<String>),
    /// A `-`/`*`/`+` unordered list item; content still has inline
    /// styling to parse.
    Bullet(String),
    /// An ordered list item: the number exactly as typed (never
    /// renumbered, since each line is scanned independently -- see
    /// `parse_ordered`) and its content.
    Ordered(String, String),
    /// A `>` blockquote line, with its nesting depth (repeated `>`s --
    /// see `parse_blockquote`) and its content.
    Blockquote(usize, String),
    /// Any other non-empty line, with inline styling still to parse.
    Paragraph(String),
    /// An empty source line, rendered as a blank row.
    Blank,
}

/// Renders `text` (a chat message's raw source) into rows of styled spans:
/// headings, paragraphs, list items, and blockquotes are word-wrapped
/// (inline `**bold**`/`*italic*`/`` `code` `` styling preserved across the
/// wrap), code block lines are character-wrapped since code must never be
/// reflowed (see `wrap_chars`). Always returns at least one row, mirroring
/// `textwrap::wrap`'s behavior on an empty string.
pub fn render(text: &str, width: usize) -> Vec<Vec<Span<'static>>> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for block in parse_blocks(text) {
        match block {
            Block::Blank => rows.push(Vec::new()),
            Block::Heading(level, content) => {
                rows.extend(render_line(
                    &content,
                    width,
                    "",
                    Style::default(),
                    heading_style(level),
                ));
            }
            Block::Code(lines) => {
                for line in lines {
                    for chunk in wrap_chars(&line, width) {
                        rows.push(vec![Span::styled(chunk, code_style())]);
                    }
                }
            }
            Block::Bullet(content) => {
                rows.extend(render_line(
                    &content,
                    width,
                    "\u{2022} ",
                    Style::default(),
                    Style::default(),
                ));
            }
            Block::Ordered(number, content) => {
                let marker = format!("{number}. ");
                rows.extend(render_line(
                    &content,
                    width,
                    &marker,
                    Style::default(),
                    Style::default(),
                ));
            }
            Block::Blockquote(depth, content) => {
                let marker = "\u{2502} ".repeat(depth);
                rows.extend(render_line(
                    &content,
                    width,
                    &marker,
                    quote_bar_style(),
                    quote_style(),
                ));
            }
            Block::Paragraph(line) => {
                rows.extend(render_line(
                    &line,
                    width,
                    "",
                    Style::default(),
                    Style::default(),
                ));
            }
        }
    }
    if rows.is_empty() {
        rows.push(Vec::new());
    }
    rows
}

/// Renders one source line's inline-tokenized content, wrapped to `width`
/// minus `marker`'s width, with `marker` (styled `marker_style`)
/// prefixing the first row and blank padding of the same width on
/// continuation rows -- shared by every block that's just "one line of
/// inline-styled text," optionally with a leading marker. An empty
/// `marker` (headings, plain paragraphs) adds no prefix at all, so
/// wrapping and the resulting spans are unaffected. `content_style` is
/// patched *under* each inline span's own style (bold/italic/code), so
/// e.g. a heading or blockquote can apply a uniform base style while
/// still layering its own emphasis on top.
fn render_line(
    content: &str,
    width: usize,
    marker: &str,
    marker_style: Style,
    content_style: Style,
) -> Vec<Vec<Span<'static>>> {
    let marker_width = marker.chars().count();
    let content_width = width.saturating_sub(marker_width).max(1);
    let runs: Vec<(String, Style)> = tokenize_inline(content)
        .into_iter()
        .map(|(text, style)| (text, content_style.patch(style)))
        .collect();
    let mut wrapped = pack_words(split_into_words(flatten(&runs)), content_width);
    if wrapped.is_empty() {
        wrapped.push(Vec::new());
    }

    let blank_prefix = " ".repeat(marker_width);
    wrapped
        .into_iter()
        .enumerate()
        .map(|(index, row)| {
            let mut full_row = if index == 0 && !marker.is_empty() {
                vec![Span::styled(marker.to_string(), marker_style)]
            } else if index > 0 && !blank_prefix.is_empty() {
                vec![Span::raw(blank_prefix.clone())]
            } else {
                Vec::new()
            };
            full_row.extend(row);
            full_row
        })
        .collect()
}

/// Scans `text` into `Block`s, one per source line -- split with the same
/// `split('\n')` `textwrap::wrap` itself uses, so a message with no
/// markdown syntax at all reflows identically to before this module
/// replaced the plain `textwrap::wrap` call in `ui::push_chat_rows`. A
/// line of 1-3 `#`s plus a space starts a heading; a fence line toggles
/// code-block mode until the next one (or the end of the text, for an
/// unterminated fence -- still shown rather than silently dropped); a
/// `>` starts a blockquote; a `-`/`*`/`+` plus a space starts an
/// unordered list item; digits plus `. ` start an ordered one.
fn parse_blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut in_code = false;
    let mut code_lines: Vec<String> = Vec::new();

    for line in text.split('\n') {
        if in_code {
            if is_fence(line) {
                blocks.push(Block::Code(std::mem::take(&mut code_lines)));
                in_code = false;
            } else {
                code_lines.push(line.to_string());
            }
            continue;
        }
        if is_fence(line) {
            in_code = true;
            continue;
        }
        if let Some((level, content)) = parse_heading(line) {
            blocks.push(Block::Heading(level, content.to_string()));
        } else if let Some((depth, content)) = parse_blockquote(line) {
            blocks.push(Block::Blockquote(depth, content.to_string()));
        } else if let Some(content) = parse_bullet(line) {
            blocks.push(Block::Bullet(content.to_string()));
        } else if let Some((number, content)) = parse_ordered(line) {
            blocks.push(Block::Ordered(number.to_string(), content.to_string()));
        } else if line.is_empty() {
            blocks.push(Block::Blank);
        } else {
            blocks.push(Block::Paragraph(line.to_string()));
        }
    }
    if in_code {
        blocks.push(Block::Code(code_lines));
    }
    blocks
}

/// A fenced code block delimiter -- a line starting with three backticks.
/// A language tag after the fence (e.g. "```rust") is accepted but
/// ignored, since no syntax highlighting is implemented.
fn is_fence(line: &str) -> bool {
    line.starts_with("```")
}

/// Recognizes a `#`/`##`/`###` heading line, returning its level and
/// trimmed content. Four or more `#`s, or no space after them (or nothing
/// after the space), is left as a plain paragraph -- deeper heading
/// levels aren't worth a distinct style in a chat message.
fn parse_heading(line: &str) -> Option<(u8, &str)> {
    let hashes = line.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 3 {
        return None;
    }
    let content = line[hashes..].strip_prefix(' ')?;
    if content.is_empty() {
        return None;
    }
    Some((hashes as u8, content))
}

/// Recognizes a `-`/`*`/`+` unordered list item (the marker followed by a
/// space), returning its content.
fn parse_bullet(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix('-')
        .or_else(|| line.strip_prefix('*'))
        .or_else(|| line.strip_prefix('+'))?;
    rest.strip_prefix(' ')
}

/// Recognizes an ordered list item: one or more digits followed by `. `,
/// returning the number text (used verbatim as typed -- items are never
/// renumbered, since each line is scanned independently of the others)
/// and the content after it. A decimal like "1.5" is correctly left as a
/// plain paragraph, since there's no space right after the `.`.
fn parse_ordered(line: &str) -> Option<(&str, &str)> {
    let digit_count = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count == 0 {
        return None;
    }
    let (number, rest) = line.split_at(digit_count);
    let content = rest.strip_prefix(". ")?;
    Some((number, content))
}

/// Recognizes a `>` blockquote line, counting consecutive `>` markers
/// (each optionally followed by one space, so both ">>nested" and
/// "> > nested" work) for its nesting depth, and returning the content
/// after them.
fn parse_blockquote(line: &str) -> Option<(usize, &str)> {
    let mut rest = line.strip_prefix('>')?;
    let mut depth = 1;
    loop {
        let after_space = rest.strip_prefix(' ').unwrap_or(rest);
        match after_space.strip_prefix('>') {
            Some(next) => {
                rest = next;
                depth += 1;
            }
            None => {
                rest = after_space;
                break;
            }
        }
    }
    Some((depth, rest))
}

/// Heading style: bold accent color for every level, with a small extra
/// touch (underline for `#`, italic for `###`) so the three levels are
/// still visually distinguishable at a glance.
fn heading_style(level: u8) -> Style {
    let style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    match level {
        1 => style.add_modifier(Modifier::UNDERLINED),
        3 => style.add_modifier(Modifier::ITALIC),
        _ => style,
    }
}

fn code_style() -> Style {
    Style::default().fg(Color::Magenta)
}

fn bold_style() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

fn italic_style() -> Style {
    Style::default().add_modifier(Modifier::ITALIC)
}

/// Style applied to a blockquote's own text -- italic, with no color
/// change, so it reads as quoted without looking like a system notice
/// (which uses a dim gray italic elsewhere in `ui.rs`).
fn quote_style() -> Style {
    Style::default().add_modifier(Modifier::ITALIC)
}

/// Style applied to a blockquote's `\u{2502}` bar marker(s) -- dim, so the
/// bar reads as a structural rail rather than part of the quoted text.
fn quote_bar_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Whether `c` counts as part of an identifier -- used to keep `_..._`
/// italics from firing inside `snake_case_names`, see `tokenize_inline`.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Scans one line of paragraph/heading text into styled runs, recognizing
/// `` `code` ``, `**bold**`, and `*italic*`/`_italic_`. Unterminated or
/// empty (e.g. "````") markers are left as literal text, and markers
/// aren't parsed recursively inside another span's content -- nested
/// inline styles are out of scope. `_..._` additionally requires a
/// non-word character (or line start/end) immediately outside both
/// delimiters, so common snake_case identifiers and filenames (e.g.
/// `channel_registry.rs`) don't get misread as italics.
fn tokenize_inline(line: &str) -> Vec<(String, Style)> {
    let chars: Vec<char> = line.chars().collect();
    let mut runs: Vec<(String, Style)> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;
    while i < chars.len() {
        let matched = match chars[i] {
            '`' => try_span(&chars, i, 1, '`', false, code_style()),
            '*' if chars.get(i + 1) == Some(&'*') => {
                try_span(&chars, i, 2, '*', false, bold_style())
            }
            '*' => try_span(&chars, i, 1, '*', false, italic_style()),
            '_' => try_span(&chars, i, 1, '_', true, italic_style()),
            _ => None,
        };
        match matched {
            Some((content, style, next)) => {
                if !plain.is_empty() {
                    runs.push((std::mem::take(&mut plain), Style::default()));
                }
                runs.push((content, style));
                i = next;
            }
            None => {
                plain.push(chars[i]);
                i += 1;
            }
        }
    }
    if !plain.is_empty() {
        runs.push((plain, Style::default()));
    }
    runs
}

/// Tries to match a delimited span opening at `chars[i]`, where the
/// delimiter is `marker` repeated `marker_len` times. `guard_word_chars`
/// rejects the match if a word character (see `is_word_char`) sits
/// immediately outside either delimiter -- used for `_..._` only.
/// Returns the un-styled content, its style, and the index just past the
/// closing delimiter.
fn try_span(
    chars: &[char],
    i: usize,
    marker_len: usize,
    marker: char,
    guard_word_chars: bool,
    style: Style,
) -> Option<(String, Style, usize)> {
    if guard_word_chars && i > 0 && is_word_char(chars[i - 1]) {
        return None;
    }
    let open_end = i + marker_len;
    let close_start = find_marker(chars, open_end, marker, marker_len)?;
    if close_start == open_end {
        return None; // empty span, e.g. "``" or "****" -- treat as literal
    }
    let close_end = close_start + marker_len;
    if guard_word_chars && chars.get(close_end).is_some_and(|&c| is_word_char(c)) {
        return None;
    }
    Some((
        chars[open_end..close_start].iter().collect(),
        style,
        close_end,
    ))
}

/// Finds the next run of `marker_len` consecutive `marker` characters at
/// or after `from`.
fn find_marker(chars: &[char], from: usize, marker: char, marker_len: usize) -> Option<usize> {
    let upper = chars.len().checked_sub(marker_len)?;
    (from..=upper).find(|&i| chars[i..i + marker_len].iter().all(|&c| c == marker))
}

/// Flattens styled runs into a per-character tape, so word-splitting
/// doesn't need to care which run a whitespace/word boundary happened to
/// fall in (e.g. `**bo**ld` is one word spanning two styles).
fn flatten(runs: &[(String, Style)]) -> Vec<(char, Style)> {
    runs.iter()
        .flat_map(|(text, style)| text.chars().map(move |c| (c, *style)))
        .collect()
}

/// One wrappable unit for the paragraph/heading packer: a maximal run of
/// non-whitespace chars plus any whitespace immediately following it --
/// mirrors how `textwrap` attaches trailing whitespace to the preceding
/// word, dropping it only where a word ends up at a row's end (see
/// `pack_words`/`trim_trailing`). A purely-whitespace prefix (e.g. a
/// message starting with spaces) becomes its own leading `Word`.
struct Word {
    pieces: Vec<(String, Style)>,
    width: usize,
}

/// Splits a per-character styled tape into `Word`s -- see `Word`'s doc
/// comment for the whitespace-attachment rule this preserves.
fn split_into_words(tape: Vec<(char, Style)>) -> Vec<Word> {
    let mut words = Vec::new();
    let mut current: Vec<(char, Style)> = Vec::new();
    let mut has_word = false;

    for (ch, style) in tape {
        let is_space = ch.is_whitespace();
        let trailing_space_pending = current.last().is_some_and(|(c, _)| c.is_whitespace());
        if !is_space && has_word && trailing_space_pending {
            words.push(finish_word(std::mem::take(&mut current)));
            has_word = false;
        } else if !is_space && !has_word && !current.is_empty() {
            words.push(finish_word(std::mem::take(&mut current)));
        }
        current.push((ch, style));
        if !is_space {
            has_word = true;
        }
    }
    if !current.is_empty() {
        words.push(finish_word(current));
    }
    words
}

/// Coalesces a word's per-character tape into same-style runs.
fn finish_word(chars: Vec<(char, Style)>) -> Word {
    let width = chars.len();
    let mut pieces: Vec<(String, Style)> = Vec::new();
    for (ch, style) in chars {
        match pieces.last_mut() {
            Some((text, last_style)) if *last_style == style => text.push(ch),
            _ => pieces.push((ch.to_string(), style)),
        }
    }
    Word { pieces, width }
}

/// Greedily packs `words` into rows of at most `width` columns (mirroring
/// `textwrap::wrap`'s word-wrap, but style-aware), hard-breaking any
/// single word wider than `width` on its own (e.g. a long URL, via
/// `chunk_word`) so one long token can't force an overflowing row.
fn pack_words(words: Vec<Word>, width: usize) -> Vec<Vec<Span<'static>>> {
    let mut rows: Vec<Vec<(String, Style)>> = Vec::new();
    let mut current: Vec<(String, Style)> = Vec::new();
    let mut current_width = 0usize;

    for word in words {
        if word.width > width {
            if !current.is_empty() {
                rows.push(trim_trailing(std::mem::take(&mut current)));
                current_width = 0;
            }
            let mut chunks = chunk_word(&word, width);
            let last = chunks.pop();
            rows.extend(chunks);
            if let Some(last) = last {
                current_width = last.iter().map(|(text, _)| text.chars().count()).sum();
                current = last;
            }
            continue;
        }
        if !current.is_empty() && current_width + word.width > width {
            rows.push(trim_trailing(std::mem::take(&mut current)));
            current_width = 0;
        }
        current.extend(word.pieces);
        current_width += word.width;
    }
    if !current.is_empty() {
        rows.push(trim_trailing(current));
    }
    rows.into_iter().map(build_spans).collect()
}

/// Breaks an oversized word's styled pieces into `width`-wide chunks,
/// preserving each piece's style across the break.
fn chunk_word(word: &Word, width: usize) -> Vec<Vec<(String, Style)>> {
    let mut chunks = Vec::new();
    let mut current: Vec<(String, Style)> = Vec::new();
    let mut current_width = 0usize;
    for (text, style) in &word.pieces {
        for ch in text.chars() {
            if current_width == width {
                chunks.push(std::mem::take(&mut current));
                current_width = 0;
            }
            match current.last_mut() {
                Some((t, s)) if *s == *style => t.push(ch),
                _ => current.push((ch.to_string(), *style)),
            }
            current_width += 1;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// Drops trailing whitespace from a row's last piece(s) -- `textwrap`
/// does the same (see its doc comment's "Leading and Trailing
/// Whitespace" section) so a wrapped row never ends with dangling spaces.
fn trim_trailing(mut pieces: Vec<(String, Style)>) -> Vec<(String, Style)> {
    while let Some((text, _)) = pieces.last_mut() {
        let trimmed_len = text.trim_end().len();
        if trimmed_len == text.len() {
            break;
        }
        if trimmed_len == 0 {
            pieces.pop();
        } else {
            text.truncate(trimmed_len);
            break;
        }
    }
    pieces
}

fn build_spans(pieces: Vec<(String, Style)>) -> Vec<Span<'static>> {
    pieces
        .into_iter()
        .map(|(text, style)| Span::styled(text, style))
        .collect()
}

/// Splits `text` into `width`-wide chunks, breaking mid-word if needed.
/// Used for code blocks (which must never be reflowed) and by
/// `ui::build_message_rows` for unbreakable system lines like invite
/// tickets, which are one long token with no natural break points at all.
pub(crate) fn wrap_chars(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_text(row: &[Span<'static>]) -> String {
        row.iter().map(|span| span.content.as_ref()).collect()
    }

    #[test]
    fn parse_blocks_recognizes_heading_levels_one_through_three() {
        assert_eq!(
            parse_blocks("# One\n## Two\n### Three"),
            vec![
                Block::Heading(1, "One".to_string()),
                Block::Heading(2, "Two".to_string()),
                Block::Heading(3, "Three".to_string()),
            ]
        );
    }

    #[test]
    fn parse_blocks_treats_four_hashes_as_a_paragraph() {
        assert_eq!(
            parse_blocks("#### not a heading"),
            vec![Block::Paragraph("#### not a heading".to_string())]
        );
    }

    #[test]
    fn parse_blocks_requires_a_space_after_the_hashes() {
        assert_eq!(
            parse_blocks("#hashtag"),
            vec![Block::Paragraph("#hashtag".to_string())]
        );
    }

    #[test]
    fn parse_blocks_collects_a_fenced_code_block() {
        assert_eq!(
            parse_blocks("before\n```rust\nfn main() {}\n```\nafter"),
            vec![
                Block::Paragraph("before".to_string()),
                Block::Code(vec!["fn main() {}".to_string()]),
                Block::Paragraph("after".to_string()),
            ]
        );
    }

    #[test]
    fn parse_blocks_still_shows_an_unterminated_fence() {
        assert_eq!(
            parse_blocks("```\nno closing fence"),
            vec![Block::Code(vec!["no closing fence".to_string()])]
        );
    }

    #[test]
    fn parse_blocks_treats_an_empty_line_as_blank() {
        assert_eq!(
            parse_blocks("one\n\ntwo"),
            vec![
                Block::Paragraph("one".to_string()),
                Block::Blank,
                Block::Paragraph("two".to_string()),
            ]
        );
    }

    #[test]
    fn parse_blocks_recognizes_unordered_list_markers() {
        assert_eq!(
            parse_blocks("- one\n* two\n+ three"),
            vec![
                Block::Bullet("one".to_string()),
                Block::Bullet("two".to_string()),
                Block::Bullet("three".to_string()),
            ]
        );
    }

    #[test]
    fn parse_blocks_does_not_treat_a_bare_dash_as_a_bullet() {
        // No space after the marker -- not a list item.
        assert_eq!(
            parse_blocks("-not a list"),
            vec![Block::Paragraph("-not a list".to_string())]
        );
    }

    #[test]
    fn parse_blocks_recognizes_an_ordered_list_item() {
        assert_eq!(
            parse_blocks("1. first\n42. later"),
            vec![
                Block::Ordered("1".to_string(), "first".to_string()),
                Block::Ordered("42".to_string(), "later".to_string()),
            ]
        );
    }

    #[test]
    fn parse_blocks_does_not_misread_a_decimal_number_as_a_list() {
        assert_eq!(
            parse_blocks("v1.5 released"),
            vec![Block::Paragraph("v1.5 released".to_string())]
        );
    }

    #[test]
    fn parse_blocks_recognizes_a_blockquote() {
        assert_eq!(
            parse_blocks("> quoted text"),
            vec![Block::Blockquote(1, "quoted text".to_string())]
        );
    }

    #[test]
    fn parse_blocks_recognizes_a_blockquote_with_no_space_after_the_marker() {
        assert_eq!(
            parse_blocks(">quoted"),
            vec![Block::Blockquote(1, "quoted".to_string())]
        );
    }

    #[test]
    fn parse_blocks_recognizes_a_nested_blockquote() {
        assert_eq!(
            parse_blocks("> > deeply quoted"),
            vec![Block::Blockquote(2, "deeply quoted".to_string())]
        );
        assert_eq!(
            parse_blocks(">>no spaces"),
            vec![Block::Blockquote(2, "no spaces".to_string())]
        );
    }

    #[test]
    fn tokenize_inline_recognizes_bold_italic_and_code() {
        assert_eq!(
            tokenize_inline("a **b** c *d* e `f` g"),
            vec![
                ("a ".to_string(), Style::default()),
                ("b".to_string(), bold_style()),
                (" c ".to_string(), Style::default()),
                ("d".to_string(), italic_style()),
                (" e ".to_string(), Style::default()),
                ("f".to_string(), code_style()),
                (" g".to_string(), Style::default()),
            ]
        );
    }

    #[test]
    fn tokenize_inline_supports_underscore_italics() {
        assert_eq!(
            tokenize_inline("hello _world_ there"),
            vec![
                ("hello ".to_string(), Style::default()),
                ("world".to_string(), italic_style()),
                (" there".to_string(), Style::default()),
            ]
        );
    }

    #[test]
    fn tokenize_inline_leaves_an_unterminated_marker_literal() {
        assert_eq!(
            tokenize_inline("just *asterisk with no close"),
            vec![("just *asterisk with no close".to_string(), Style::default())]
        );
    }

    #[test]
    fn tokenize_inline_leaves_empty_markers_literal() {
        assert_eq!(
            tokenize_inline("****"),
            vec![("****".to_string(), Style::default())]
        );
    }

    #[test]
    fn tokenize_inline_does_not_misread_underscores_in_identifiers() {
        assert_eq!(
            tokenize_inline("see channel_registry.rs and self_id"),
            vec![(
                "see channel_registry.rs and self_id".to_string(),
                Style::default()
            )]
        );
    }

    #[test]
    fn render_wraps_a_heading_with_its_style() {
        let rows = render("# Title", 80);
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "Title");
        assert_eq!(rows[0][0].style, heading_style(1));
    }

    #[test]
    fn render_keeps_code_block_lines_unwrapped_and_styled() {
        let rows = render("```\nlet x = 1;\n```", 80);
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "let x = 1;");
        assert_eq!(rows[0][0].style, code_style());
    }

    #[test]
    fn render_hard_wraps_a_code_line_longer_than_the_width() {
        let rows = render("```\nabcdefghij\n```", 4);
        let joined: String = rows.iter().map(|row| row_text(row)).collect();
        assert_eq!(joined, "abcdefghij");
        assert!(rows.iter().all(|row| row_text(row).chars().count() <= 4));
    }

    #[test]
    fn render_preserves_a_blank_line_between_paragraphs() {
        let rows = render("one\n\ntwo", 80);
        assert_eq!(
            rows.iter().map(|row| row_text(row)).collect::<Vec<_>>(),
            vec!["one".to_string(), String::new(), "two".to_string()]
        );
    }

    #[test]
    fn render_word_wraps_plain_text_like_textwrap() {
        // No markdown syntax at all -- should reflow exactly the way
        // `textwrap::wrap` already did before this module replaced it.
        let rows = render("Foo   bar baz", 10);
        assert_eq!(
            rows.iter().map(|row| row_text(row)).collect::<Vec<_>>(),
            vec!["Foo   bar".to_string(), "baz".to_string()]
        );
    }

    #[test]
    fn render_keeps_bold_styling_across_a_wrap_boundary() {
        let rows = render("plain **bold word** more", 12);
        let flattened: Vec<(String, Style)> = rows
            .iter()
            .flat_map(|row| {
                row.iter()
                    .map(|span| (span.content.to_string(), span.style))
            })
            .collect();
        assert!(
            flattened
                .iter()
                .any(|(text, style)| text.contains("bold") && *style == bold_style())
        );
    }

    #[test]
    fn render_never_returns_an_empty_row_list() {
        assert_eq!(render("", 80), vec![Vec::<Span<'static>>::new()]);
    }

    #[test]
    fn render_renders_a_bullet_with_its_marker() {
        let rows = render("- an item", 80);
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "\u{2022} an item");
    }

    #[test]
    fn render_hanging_indents_a_wrapped_bullet_under_its_text() {
        let rows = render("- a longer item that wraps", 12);
        assert!(
            rows.len() >= 2,
            "expected the item to wrap onto multiple rows"
        );
        assert!(row_text(&rows[0]).starts_with("\u{2022} "));
        assert!(
            row_text(&rows[1]).starts_with("  "),
            "continuation row should line up under the text, not the bullet: {:?}",
            row_text(&rows[1])
        );
    }

    #[test]
    fn render_renders_an_ordered_item_with_its_number() {
        let rows = render("1. first item", 80);
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "1. first item");
    }

    #[test]
    fn render_supports_inline_styling_inside_a_list_item() {
        let rows = render("- **bold** item", 80);
        assert_eq!(rows.len(), 1);
        let bold_span = rows[0]
            .iter()
            .find(|span| span.content.as_ref() == "bold")
            .expect("a bold span");
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn render_renders_a_blockquote_with_a_bar_and_italic_text() {
        let rows = render("> quoted", 80);
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "\u{2502} quoted");
        let bar_span = &rows[0][0];
        assert_eq!(bar_span.content.as_ref(), "\u{2502} ");
        assert_eq!(bar_span.style.fg, Some(Color::DarkGray));
        let text_span = rows[0].last().unwrap();
        assert_eq!(text_span.content.as_ref(), "quoted");
        assert!(text_span.style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn render_renders_a_nested_blockquote_with_a_double_bar() {
        let rows = render(">> deeply quoted", 80);
        assert_eq!(rows.len(), 1);
        assert!(row_text(&rows[0]).starts_with("\u{2502} \u{2502} "));
    }

    #[test]
    fn render_a_bare_bullet_marker_shows_just_the_marker() {
        let rows = render("-  ", 80);
        assert_eq!(rows.len(), 1);
        // Trailing whitespace is trimmed like any other wrapped row, so a
        // content-less bullet still shows exactly its own marker.
        assert_eq!(row_text(&rows[0]), "\u{2022} ");
    }

    #[test]
    fn wrap_chars_breaks_long_text_into_fixed_width_chunks() {
        assert_eq!(
            wrap_chars("abcdefgh", 3),
            vec!["abc".to_string(), "def".to_string(), "gh".to_string()]
        );
    }

    #[test]
    fn wrap_chars_returns_one_empty_chunk_for_empty_text() {
        assert_eq!(wrap_chars("", 5), vec![String::new()]);
    }
}
