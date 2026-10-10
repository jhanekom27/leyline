//! Overlays clickable [OSC 8] hyperlinks onto bare URLs that have already
//! been rendered into terminal cells -- see `ui::render_messages`, the only
//! caller.
//!
//! Ratatui's `Span`/`Style` have no concept of a hyperlink (open upstream
//! request: <https://github.com/ratatui/ratatui/issues/1028>), and
//! `Span::styled_graphemes` filters out any grapheme that contains a
//! control character. That means embedding a raw OSC 8 escape sequence
//! (which starts with `ESC`) in a `Span`'s content and rendering it through
//! the normal `Line`/`List` path gets every lone `ESC` silently dropped,
//! while the sequence's otherwise-printable bytes (`]8;;...`) leak through
//! as visible garbage.
//!
//! `linkify` sidesteps this by running *after* a frame's widgets have
//! already rendered: it rereads each row's plain text straight back out of
//! the `Buffer`'s cells, finds bare URLs (`find_links`), and patches just
//! the first and last cell of each match to carry the OSC 8 open/close
//! sequence. A terminal keeps a hyperlink "live" across any plain text
//! between those two sequences, so the cells in between need no change at
//! all. Both patched cells are tagged `CellDiffOption::ForcedWidth(1)`,
//! since the escape bytes would otherwise inflate the cell's computed text
//! width far past the single terminal column it actually occupies.
//!
//! No terminal-capability detection is needed either: a terminal that
//! doesn't understand OSC 8 discards the unrecognized sequence up to its
//! terminator and is left showing exactly the plain text that was already
//! there.
//!
//! [OSC 8]: https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda

use std::num::NonZeroU16;

use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::Rect;

/// URL schemes recognized for bare-URL autolinking. Deliberately narrow --
/// scheme-less forms like `www.example.com` are ambiguous with ordinary
/// prose and are left as plain text.
const SCHEMES: [&str; 2] = ["http://", "https://"];

/// Start of an OSC 8 hyperlink: `ESC ] 8 ; ;`.
const OSC8_START: &str = "\u{1b}]8;;";
/// String Terminator (`ESC \`), closes any OSC sequence.
const ST: &str = "\u{1b}\\";

const FORCED_WIDTH_ONE: CellDiffOption = CellDiffOption::ForcedWidth(NonZeroU16::MIN);

/// Scans every row of `area`, as already rendered into `buffer`, for bare
/// URLs and overlays OSC 8 hyperlinks onto them. Call this after rendering
/// the widget(s) that filled `area`, since it reads the plain text those
/// widgets already wrote there.
pub fn linkify(buffer: &mut Buffer, area: Rect) {
    for y in area.y..area.y.saturating_add(area.height) {
        linkify_row(buffer, area.x, area.width, y);
    }
}

fn linkify_row(buffer: &mut Buffer, x: u16, width: u16, y: u16) {
    let text = row_text(buffer, x, width, y);
    for (col, link_width, url) in find_links(&text) {
        apply_hyperlink(buffer, x + col as u16, y, link_width, &url);
    }
}

/// Reconstructs a row's plain text, one character per cell. URLs are ASCII,
/// so this always keeps columns aligned for the text `find_links` looks
/// for, even though it under-reads any wide (double-width) grapheme
/// elsewhere in the row.
fn row_text(buffer: &Buffer, x: u16, width: u16, y: u16) -> String {
    (0..width)
        .map(|dx| {
            buffer
                .cell((x + dx, y))
                .and_then(|cell| cell.symbol().chars().next())
                .unwrap_or(' ')
        })
        .collect()
}

/// Scans already-wrapped plain text for bare URLs, returning each match's
/// starting character column, character width, and URL text. See
/// `find_link_at` for what counts as a match.
fn find_links(text: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut links = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match find_link_at(&chars, i) {
            Some((width, url)) => {
                links.push((i, width, url));
                i += width;
            }
            None => i += 1,
        }
    }
    links
}

/// Tries to match a bare URL starting exactly at `chars[i]`: an `http://`
/// or `https://` scheme, preceded by whitespace or the start of the text
/// (so e.g. `(https://example.com)` isn't linked, since the URL doesn't
/// start at a word boundary), consumed up to the next whitespace and then
/// trimmed of a small set of trailing sentence punctuation (`.,;:!?`) so a
/// URL at the end of a sentence doesn't pull that punctuation into the
/// link.
///
/// Rejected if, even after trimming, the match still runs all the way to
/// the end of `chars`. `chars` is always exactly one terminal row (see
/// `row_text`), and a URL wider than the row would have been hard-wrapped
/// onto the next one (see `markdown::chunk_word`), leaving only a
/// truncated fragment here with no trailing punctuation to trim back.
/// That's indistinguishable from a URL that just happens to end on the
/// row's last column, so both are skipped rather than risk a clickable
/// link to a truncated, wrong address.
fn find_link_at(chars: &[char], i: usize) -> Option<(usize, String)> {
    if i > 0 && !chars[i - 1].is_whitespace() {
        return None;
    }
    let scheme_len = SCHEMES
        .into_iter()
        .find(|scheme| matches_at(chars, i, scheme))?
        .chars()
        .count();
    let scheme_end = i + scheme_len;
    let mut end = scheme_end;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }
    while end > scheme_end && is_trailing_punctuation(chars[end - 1]) {
        end -= 1;
    }
    if end == chars.len() {
        return None;
    }
    Some((end - i, chars[i..end].iter().collect()))
}

fn matches_at(chars: &[char], i: usize, pattern: &str) -> bool {
    pattern
        .chars()
        .enumerate()
        .all(|(offset, c)| chars.get(i + offset) == Some(&c))
}

fn is_trailing_punctuation(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '!' | '?')
}

/// Patches the cells spanning `[x, x + width)` on row `y` so the first
/// opens an OSC 8 hyperlink to `url` and the last closes it, leaving every
/// cell's existing (already-rendered) symbol and style otherwise untouched
/// -- see the module doc comment for why only the two boundary cells need
/// it.
fn apply_hyperlink(buffer: &mut Buffer, x: u16, y: u16, width: usize, url: &str) {
    if width == 0 {
        return;
    }
    let last_x = x + (width - 1) as u16;
    if last_x == x {
        patch_cell(buffer, x, y, |symbol| {
            format!("{OSC8_START}{url}{ST}{symbol}{OSC8_START}{ST}")
        });
        return;
    }
    patch_cell(buffer, x, y, |symbol| {
        format!("{OSC8_START}{url}{ST}{symbol}")
    });
    patch_cell(buffer, last_x, y, |symbol| {
        format!("{symbol}{OSC8_START}{ST}")
    });
}

fn patch_cell(buffer: &mut Buffer, x: u16, y: u16, wrap: impl FnOnce(&str) -> String) {
    let Some(cell) = buffer.cell_mut((x, y)) else {
        return;
    };
    let wrapped = wrap(cell.symbol());
    cell.set_symbol(&wrapped);
    cell.set_diff_option(FORCED_WIDTH_ONE);
}

#[cfg(test)]
mod tests {
    use ratatui::style::Style;

    use super::*;

    #[test]
    fn find_links_detects_a_bare_url_preceded_by_whitespace() {
        assert_eq!(
            find_links("check https://example.com now"),
            vec![(6, 19, "https://example.com".to_string())]
        );
    }

    #[test]
    fn find_links_recognizes_the_plain_http_scheme_too() {
        assert_eq!(
            find_links("see http://example.com here"),
            vec![(4, 18, "http://example.com".to_string())]
        );
    }

    #[test]
    fn find_links_matches_a_url_at_the_very_start_of_the_text() {
        assert_eq!(
            find_links("https://example.com is great"),
            vec![(0, 19, "https://example.com".to_string())]
        );
    }

    #[test]
    fn find_links_trims_trailing_sentence_punctuation() {
        assert_eq!(
            find_links("see https://example.com."),
            vec![(4, 19, "https://example.com".to_string())]
        );
    }

    #[test]
    fn find_links_ignores_a_url_not_preceded_by_whitespace() {
        assert_eq!(find_links("(https://example.com)"), vec![]);
    }

    #[test]
    fn find_links_skips_a_url_that_runs_to_the_end_of_the_row() {
        // Indistinguishable here from a URL hard-wrapped onto the next
        // row -- see `find_link_at`'s doc comment.
        assert_eq!(find_links("https://example.com"), vec![]);
    }

    #[test]
    fn find_links_finds_more_than_one_url_in_the_same_row() {
        assert_eq!(
            find_links("https://a.com and https://b.com "),
            vec![
                (0, 13, "https://a.com".to_string()),
                (18, 13, "https://b.com".to_string()),
            ]
        );
    }

    #[test]
    fn linkify_wraps_the_matched_url_in_an_osc_8_sequence() {
        let area = Rect::new(0, 0, 30, 1);
        let mut buffer = Buffer::empty(area);
        buffer.set_string(0, 0, "check https://example.com now", Style::default());

        linkify(&mut buffer, area);

        let first = buffer.cell((6, 0)).expect("first link cell");
        assert_eq!(first.symbol(), "\u{1b}]8;;https://example.com\u{1b}\\h");
        assert_eq!(first.diff_option, FORCED_WIDTH_ONE);

        let last = buffer.cell((24, 0)).expect("last link cell");
        assert_eq!(last.symbol(), "m\u{1b}]8;;\u{1b}\\");
        assert_eq!(last.diff_option, FORCED_WIDTH_ONE);

        let middle = buffer.cell((14, 0)).expect("middle link cell");
        assert_eq!(middle.symbol(), "e");
        assert_eq!(middle.diff_option, CellDiffOption::None);

        let outside = buffer.cell((0, 0)).expect("cell before the link");
        assert_eq!(outside.symbol(), "c");
    }

    #[test]
    fn linkify_leaves_plain_text_with_no_url_untouched() {
        let area = Rect::new(0, 0, 20, 1);
        let mut buffer = Buffer::empty(area);
        buffer.set_string(0, 0, "no links here", Style::default());
        let before = buffer.clone();

        linkify(&mut buffer, area);

        assert_eq!(buffer, before);
    }
}
