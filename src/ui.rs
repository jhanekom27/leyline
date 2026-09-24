//! Pure rendering: reads `AppState`, writes widgets to the frame. No
//! mutation, no I/O -- see concept.md's "one owner of UI state" principle.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::scrollbar;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use textwrap::wrap;

use crate::app::{AppState, Channel, TranscriptLine, hex_id};
use crate::files::human_size;
use crate::message::ChatMessage;
use crate::search::SearchResults;

/// Width of the `HH:MM` time column in a chat row.
const TIME_WIDTH: usize = 5;
/// Width of the sender-name column in a chat row; longer names are
/// truncated with an ellipsis so the `│` separator after it always lines
/// up (see `fit_name`).
const NAME_WIDTH: usize = 10;
/// Separates the name column from the message text.
const SEPARATOR: &str = " │ ";

/// Short usage reminders for every slash command, shown in the sidebar's
/// commands panel (`render_hints`) when `AppState::show_hints` is on.
/// Kept in sync with the Commands table in README.md and `run_help`'s
/// system notice.
const COMMAND_HINTS: &[&str] = &[
    "/join <name|ticket>",
    "/invite",
    "/leave [channel]",
    "/who",
    "/send <path>",
    "/save <hash>",
    "/paste (or Ctrl+V)",
    "/alias <hex> <name>",
    "/nick <name>",
    "/search <term> (/s)",
    "/hints",
    "/help",
];

/// Curated, dark-background-friendly colors used to visually separate
/// senders -- see `user_color`.
const USER_PALETTE: [Color; 8] = [
    Color::Rgb(0x5f, 0xd4, 0xc4),
    Color::Rgb(0xe0, 0xa8, 0x5a),
    Color::Rgb(0xb3, 0x9c, 0xf0),
    Color::Rgb(0x7f, 0xb3, 0xff),
    Color::Rgb(0xe8, 0x8a, 0xa3),
    Color::Rgb(0x8a, 0xd4, 0x7a),
    Color::Rgb(0xf0, 0xd4, 0x5a),
    Color::Rgb(0x9c, 0xa8, 0xf0),
];

pub fn render(frame: &mut Frame, app: &AppState) {
    let [header, body, input] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    render_body(frame, body, app);
    render_input(frame, input, app);
}

fn render_header(frame: &mut Frame, area: Rect, app: &AppState) {
    let [tabs, identity] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

    let mut spans = vec![Span::styled(
        " leyline ",
        Style::default().add_modifier(Modifier::BOLD),
    )];
    for (index, channel) in app.channels.iter().enumerate() {
        let unread_marker = if channel.has_unread && index != app.active {
            "*"
        } else {
            ""
        };
        let style = if index == app.active {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        spans.push(Span::styled(
            format!(" #{}{unread_marker} ", channel.name),
            style,
        ));
    }

    let tabs_line = Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(tabs_line, tabs);

    let identity_line = Paragraph::new(Line::from(vec![
        Span::raw(" you: "),
        Span::styled(
            hex_id(&app.self_id),
            Style::default().fg(user_color(&app.self_id)),
        ),
    ]));
    frame.render_widget(identity_line, identity);
}

fn render_body(frame: &mut Frame, area: Rect, app: &AppState) {
    let [messages, sidebar] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(22)]).areas(area);

    render_messages(frame, messages, app);
    render_sidebar(frame, sidebar, app);
}

/// Splits the RHS column into the peers list and, when `show_hints` is
/// on, a command reference panel below it (toggled via `/hints` -- see
/// `app::AppState::run_hints`). Peers keeps using whatever room is
/// available (`Constraint::Min`), same as when the hints panel is
/// hidden; the hints panel only ever takes exactly the room its fixed
/// content needs (`Constraint::Length`).
fn render_sidebar(frame: &mut Frame, area: Rect, app: &AppState) {
    if !app.show_hints {
        render_peers(frame, area, app);
        return;
    }
    let hints_height = COMMAND_HINTS.len() as u16 + 2; // +2 for the block's borders
    let [peers, hints] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(hints_height)]).areas(area);
    render_peers(frame, peers, app);
    render_hints(frame, hints);
}

fn render_messages(frame: &mut Frame, area: Rect, app: &AppState) {
    let channel = app.active();
    let visible_rows = area.height.saturating_sub(2).max(1) as usize; // minus borders
    let inner_width = (area.width as usize).saturating_sub(2).max(1); // minus borders

    // Flattened one-`Line`-per-terminal-row, not one per `TranscriptLine`
    // -- a wrapped message now spans several rows, so the scroll window
    // below has to work in rows to stay accurate.
    let mut rows = build_message_rows(app, channel, inner_width);
    let total = rows.len();
    // `channel.scroll` grows unboundedly while `Up` is held (see
    // `AppState::handle_key`) since it has no way to know how many rows
    // of *wrapped* content exist -- that depends on the terminal width,
    // which is only known here. Clamp it so scrolling past the oldest
    // message just holds the view there instead of pushing every row
    // off-screen.
    let max_scroll = total.saturating_sub(visible_rows);
    let scroll = channel.scroll.min(max_scroll);
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(visible_rows);
    let items: Vec<ListItem> = rows.drain(start..end).map(ListItem::new).collect();

    let title = match &channel.search {
        Some(search) => {
            let status = if search.pending {
                ", searching full history..."
            } else {
                ""
            };
            format!(
                "#{} · search '{}' ({}{status}) · /s to clear",
                channel.name,
                search.term,
                search.messages.len()
            )
        }
        None => format!("#{} · {} peer(s)", channel.name, channel.peers.len()),
    };
    let list = List::new(items).block(
        Block::bordered()
            .title(title)
            .border_type(BorderType::Rounded),
    );
    frame.render_widget(list, area);
    render_scroll_indicator(frame, area, start, total, visible_rows);
}

/// Overlays a scroll-position thumb on the messages block's right border,
/// so it's clear at a glance how far back you've scrolled and how much
/// history is above. `start` is the same top-of-viewport row index used
/// to slice the visible window above. Only drawn once the transcript
/// actually overflows one screen (`total > visible_rows`) -- a
/// full-height thumb on a short conversation would just be noise.
fn render_scroll_indicator(
    frame: &mut Frame,
    area: Rect,
    start: usize,
    total: usize,
    visible_rows: usize,
) {
    if total <= visible_rows {
        return;
    }
    // `start` only ever ranges from 0 to `total - visible_rows` -- the
    // viewport always shows a full page (see above), never overhanging
    // past the end the way a bare `Paragraph::scroll` offset would -- so
    // `content_length` must be the count of those valid positions, not
    // `total` itself, or the thumb falls short of the bottom even when
    // `start` is already at its max (i.e. fully scrolled down).
    let scroll_positions = total.saturating_sub(visible_rows) + 1;
    let mut state = ScrollbarState::new(scroll_positions)
        .position(start)
        .viewport_content_length(visible_rows);
    let indicator = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .symbols(scrollbar::VERTICAL)
        .begin_symbol(None)
        .end_symbol(None)
        .thumb_symbol("▐")
        .thumb_style(Color::Gray)
        .track_style(Color::DarkGray);
    frame.render_stateful_widget(
        indicator,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut state,
    );
}

/// Renders `channel`'s transcript into rendered rows: a colored rail, time,
/// sender name, separator, and message text for `Chat` lines (see the
/// module-level column layout), or the existing dim/italic notice style for
/// `System` lines. Sender grouping -- blanking the time/name (rail stays)
/// on a run of messages from the same person, or on a message's own
/// word-wrapped continuation lines -- is computed here since it depends on
/// iterating the transcript in order.
///
/// Delegates to `build_search_rows` when `channel` has an active `/search`
/// (see `app::Channel::search`), rendering its results in place of the
/// normal transcript.
fn build_message_rows(app: &AppState, channel: &Channel, inner_width: usize) -> Vec<Line<'static>> {
    if let Some(search) = &channel.search {
        return build_search_rows(app, search, inner_width);
    }

    let layout = row_layout(inner_width);
    let mut rows = Vec::new();
    let mut last_chat_sender: Option<[u8; 32]> = None;
    for line in &channel.messages {
        match line {
            TranscriptLine::Chat(message) => {
                let is_first_of_group = last_chat_sender != Some(message.sender);
                if is_first_of_group && last_chat_sender.is_some() {
                    // A blank row between two different senders' blocks,
                    // so consecutive messages from the same sender read as
                    // one continuous rail, while a new sender is set off
                    // at a glance instead of just butting up against it.
                    rows.push(Line::from(""));
                }
                last_chat_sender = Some(message.sender);
                push_chat_rows(&mut rows, app, message, is_first_of_group, &layout, None);
            }
            TranscriptLine::System(text) => {
                // A notice breaks up a run of grouped messages -- the next
                // `Chat` line should show its header even if it's from the
                // same sender as before the notice.
                last_chat_sender = None;
                // System lines can be long (e.g. invite tickets, which are
                // one unbroken token with no spaces to wrap on), and
                // `List` doesn't wrap on its own -- without this, anything
                // wider than the pane would just get clipped and
                // unreadable/uncopyable.
                let style = Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC);
                for chunk in wrap_chars(&format!("* {text}"), inner_width) {
                    rows.push(Line::from(Span::styled(chunk, style)));
                }
            }
        }
    }
    rows
}

/// Renders a channel's active `/search` results in place of its normal
/// transcript -- reuses the same per-message formatting
/// (`push_chat_rows`), grouped the same way, but with no `System` lines to
/// interleave since a search snapshot only ever holds chat matches.
/// Highlights the matched substring in each line (see `highlighted_spans`).
fn build_search_rows(
    app: &AppState,
    search: &SearchResults,
    inner_width: usize,
) -> Vec<Line<'static>> {
    let layout = row_layout(inner_width);
    let term_lower = search.term.to_lowercase();

    let mut rows = Vec::new();
    let mut last_sender: Option<[u8; 32]> = None;
    for message in &search.messages {
        let is_first_of_group = last_sender != Some(message.sender);
        if is_first_of_group && last_sender.is_some() {
            rows.push(Line::from(""));
        }
        last_sender = Some(message.sender);
        push_chat_rows(
            &mut rows,
            app,
            message,
            is_first_of_group,
            &layout,
            Some(&term_lower),
        );
    }
    if rows.is_empty() {
        rows.push(Line::from(Span::styled(
            "no matches",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        )));
    }
    rows
}

/// Column widths/blank fillers shared by the normal transcript and
/// search-result renderers (`build_message_rows`/`build_search_rows`) --
/// see `push_chat_rows`.
struct RowLayout {
    wrap_width: usize,
    blank_time: String,
    blank_name: String,
}

fn row_layout(inner_width: usize) -> RowLayout {
    let prefix_width = 1 + 1 + TIME_WIDTH + 1 + NAME_WIDTH + SEPARATOR.chars().count();
    RowLayout {
        wrap_width: inner_width.saturating_sub(prefix_width).max(10),
        blank_time: " ".repeat(TIME_WIDTH),
        blank_name: " ".repeat(NAME_WIDTH),
    }
}

/// Appends one chat message's rendered rows (colored rail, time, sender
/// name, separator, wrapped -- and optionally highlighted, see
/// `highlighted_spans` -- text) to `rows`. Blanks the time/name on
/// continuation lines and on a run of messages from the same sender
/// (`is_first_of_group`, computed by the caller since it depends on
/// iterating the surrounding transcript/search results in order). Shared
/// by the normal transcript (`build_message_rows`) and search results
/// (`build_search_rows`); `term_lower` is `None` for the former, which
/// never highlights anything.
fn push_chat_rows(
    rows: &mut Vec<Line<'static>>,
    app: &AppState,
    message: &ChatMessage,
    is_first_of_group: bool,
    layout: &RowLayout,
    term_lower: Option<&str>,
) {
    let color = user_color(&message.sender);
    let name = fit_name(&app.display_name(&message.sender), NAME_WIDTH);
    let time = format_time(message.ts_unix_ms);

    let caption = attachment_caption(app, message);
    let display_text = caption.as_deref().unwrap_or(&message.text);
    let mut wrapped = wrap(display_text, layout.wrap_width);
    if wrapped.is_empty() {
        wrapped.push(std::borrow::Cow::Borrowed(""));
    }
    for (index, chunk) in wrapped.iter().enumerate() {
        let show_header = index == 0 && is_first_of_group;
        let (time_span, name_span) = if show_header {
            (time.clone(), name.clone())
        } else {
            (layout.blank_time.clone(), layout.blank_name.clone())
        };
        let mut spans = vec![
            Span::styled(" ", Style::default().bg(color)),
            Span::raw(" "),
            Span::styled(time_span, Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(
                name_span,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(SEPARATOR, Style::default().fg(Color::DarkGray)),
        ];
        spans.extend(highlighted_spans(chunk, term_lower));
        rows.push(Line::from(spans));
    }
}

/// Builds the display line for a message's file attachment, if any --
/// filename, human-readable size, and (for anyone else's message) the
/// exact `/save` command to fetch it. Your own messages omit the hint
/// since the file is already local. `None` for a plain text message, in
/// which case `push_chat_rows` falls back to `message.text` unchanged.
fn attachment_caption(app: &AppState, message: &ChatMessage) -> Option<String> {
    let attachment = message.attachment.as_ref()?;
    let size = human_size(attachment.size);
    if message.sender == app.self_id {
        Some(format!("shared {} ({size})", attachment.filename))
    } else {
        let hash_prefix = &attachment.hash.to_hex()[..8];
        Some(format!(
            "shared {} ({size}) -- /save {hash_prefix} to download",
            attachment.filename
        ))
    }
}

/// Splits `text` into (segment, is_match) pairs around every
/// case-insensitive occurrence of `term_lower`, preserving `text`'s
/// original casing. Pure text logic, kept separate from `Span`
/// construction (`highlighted_spans`) so it's easy to unit test without
/// depending on ratatui's types.
///
/// Comparisons are done over `char`s, not byte offsets into a lowercased
/// `String`, since `str::to_lowercase` can change a string's byte length
/// for some Unicode input; if that happens here (rare), highlighting is
/// skipped for this line entirely rather than risking misaligned spans.
fn find_highlights(text: &str, term_lower: &str) -> Vec<(String, bool)> {
    if term_lower.is_empty() {
        return vec![(text.to_string(), false)];
    }
    let term_chars: Vec<char> = term_lower.chars().collect();
    let chars: Vec<char> = text.chars().collect();
    let lower_chars: Vec<char> = text.to_lowercase().chars().collect();
    if lower_chars.len() != chars.len() {
        return vec![(text.to_string(), false)];
    }

    let mut segments = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;
    while i + term_chars.len() <= lower_chars.len() {
        if lower_chars[i..i + term_chars.len()] == term_chars[..] {
            if i > plain_start {
                segments.push((chars[plain_start..i].iter().collect(), false));
            }
            segments.push((chars[i..i + term_chars.len()].iter().collect(), true));
            i += term_chars.len();
            plain_start = i;
        } else {
            i += 1;
        }
    }
    if plain_start < chars.len() || segments.is_empty() {
        segments.push((chars[plain_start..].iter().collect(), false));
    }
    segments
}

/// Renders `text` as one or more spans, styling matched segments (see
/// `find_highlights`) distinctly. `term_lower` is `None` for the normal
/// transcript, which never highlights anything.
fn highlighted_spans(text: &str, term_lower: Option<&str>) -> Vec<Span<'static>> {
    let Some(term_lower) = term_lower else {
        return vec![Span::raw(text.to_string())];
    };
    find_highlights(text, term_lower)
        .into_iter()
        .map(|(segment, is_match)| {
            if is_match {
                Span::styled(
                    segment,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::raw(segment)
            }
        })
        .collect()
}

/// Splits `text` into `width`-wide chunks, breaking mid-word if needed.
/// Used instead of word-wrapping because the dominant case (invite
/// tickets) is one long token with no natural break points at all.
fn wrap_chars(text: &str, width: usize) -> Vec<String> {
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

/// Pads `name` to exactly `width` characters, truncating with a trailing
/// `…` if it's longer, so the `│` separator after the name column always
/// lines up regardless of how long a nickname or petname is.
fn fit_name(name: &str, width: usize) -> String {
    let len = name.chars().count();
    if len <= width {
        format!("{name:<width$}")
    } else if width == 0 {
        String::new()
    } else {
        let truncated: String = name.chars().take(width - 1).collect();
        format!("{truncated}…")
    }
}

/// Formats a message's timestamp as a local `HH:MM`. `ts_unix_ms` (see
/// message.rs) is always a UTC epoch value; `"--:--"` covers the
/// practically-unreachable case of a timestamp chrono can't represent,
/// rather than panicking.
fn format_time(ts_unix_ms: u64) -> String {
    match DateTime::from_timestamp_millis(ts_unix_ms as i64) {
        Some(dt) => dt.with_timezone(&Local).format("%H:%M").to_string(),
        None => "--:--".to_string(),
    }
}

/// Deterministically maps an endpoint id to one of `USER_PALETTE`'s colors,
/// so a given peer -- ourselves included -- always renders in the same
/// color across the message list, peers sidebar, and header. A pure
/// function of the id rather than an assignment table, so there's no
/// per-session state to keep in sync with who's currently online; two ids
/// can land on the same color once there are more of them than the
/// palette has entries, which is an accepted tradeoff for a small, legible
/// palette.
fn user_color(id: &[u8; 32]) -> Color {
    let hash = id.iter().fold(0u32, |acc, &byte| {
        acc.wrapping_mul(31).wrapping_add(byte as u32)
    });
    USER_PALETTE[hash as usize % USER_PALETTE.len()]
}

fn render_peers(frame: &mut Frame, area: Rect, app: &AppState) {
    let channel = app.active();
    let mut items: Vec<ListItem> = channel.peers.iter().map(|id| peer_item(app, id)).collect();
    items.push(peer_item(app, &app.self_id));

    let list = List::new(items).block(
        Block::bordered()
            .title("peers")
            .border_type(BorderType::Rounded),
    );
    frame.render_widget(list, area);
}

/// One peers-sidebar row: a colored dot (`user_color`) plus the peer's
/// display name.
fn peer_item(app: &AppState, id: &[u8; 32]) -> ListItem<'static> {
    ListItem::new(Line::from(vec![
        Span::styled("● ", Style::default().fg(user_color(id))),
        Span::raw(app.display_name(id)),
    ]))
}

/// Renders the sidebar's command reference panel: one row per
/// `COMMAND_HINTS` entry, showing just the bare usage -- full
/// descriptions still live in `/help`'s system notice and the README.
fn render_hints(frame: &mut Frame, area: Rect) {
    let items: Vec<ListItem> = COMMAND_HINTS
        .iter()
        .map(|hint| ListItem::new(*hint))
        .collect();
    let list = List::new(items).block(
        Block::bordered()
            .title("commands")
            .border_type(BorderType::Rounded),
    );
    frame.render_widget(list, area);
}

fn render_input(frame: &mut Frame, area: Rect, app: &AppState) {
    let prompt = format!("{} › ", app.display_name(&app.self_id));
    let block = Block::bordered()
        .title(" type a message · /help for commands ")
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    let prompt_width = prompt.chars().count();
    let text_width = (inner.width as usize).saturating_sub(prompt_width).max(1);

    // Horizontal-scroll the input so the cursor always stays in view.
    let chars: Vec<char> = app.input.chars().collect();
    let visible_start = app.cursor.saturating_sub(text_width.saturating_sub(1));
    let visible_end = (visible_start + text_width).min(chars.len());
    let visible: String = chars[visible_start..visible_end].iter().collect();

    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(
            prompt.clone(),
            Style::default()
                .fg(user_color(&app.self_id))
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(visible),
    ]))
    .block(block);
    frame.render_widget(paragraph, area);

    let cursor_col = inner.x + (prompt_width + (app.cursor - visible_start)) as u16;
    frame.set_cursor_position((cursor_col, inner.y));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_highlights_returns_one_unmatched_segment_for_empty_term() {
        assert_eq!(
            find_highlights("hello world", ""),
            vec![("hello world".to_string(), false)]
        );
    }

    #[test]
    fn find_highlights_splits_around_a_case_insensitive_match() {
        assert_eq!(
            find_highlights("hello World", "world"),
            vec![("hello ".to_string(), false), ("World".to_string(), true)]
        );
    }

    #[test]
    fn find_highlights_handles_multiple_occurrences() {
        assert_eq!(
            find_highlights("cat cat cat", "cat"),
            vec![
                ("cat".to_string(), true),
                (" ".to_string(), false),
                ("cat".to_string(), true),
                (" ".to_string(), false),
                ("cat".to_string(), true),
            ]
        );
    }

    #[test]
    fn find_highlights_returns_one_unmatched_segment_when_nothing_matches() {
        assert_eq!(
            find_highlights("hello world", "xyz"),
            vec![("hello world".to_string(), false)]
        );
    }

    #[test]
    fn find_highlights_matches_a_term_at_the_very_end() {
        assert_eq!(
            find_highlights("say hello", "hello"),
            vec![("say ".to_string(), false), ("hello".to_string(), true)]
        );
    }

    use crate::message::FileAttachment;

    fn file_message(sender: [u8; 32], filename: &str, size: u64) -> ChatMessage {
        ChatMessage {
            v: 2,
            id: 1,
            sender,
            ts_unix_ms: 0,
            text: String::new(),
            attachment: Some(FileAttachment {
                filename: filename.to_string(),
                size,
                hash: iroh_blobs::Hash::new(filename.as_bytes()),
            }),
        }
    }

    #[test]
    fn attachment_caption_includes_a_save_hint_for_someone_elses_file() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = file_message([2; 32], "report.pdf", 2_150_000);

        let caption = attachment_caption(&app, &message).unwrap();

        assert!(caption.contains("report.pdf"), "got: {caption}");
        assert!(caption.contains("2.1 MB"), "got: {caption}");
        assert!(caption.contains("/save"), "got: {caption}");
    }

    #[test]
    fn attachment_caption_omits_the_save_hint_for_your_own_file() {
        let self_id = [1; 32];
        let app = AppState::new(self_id, vec!["general".to_string()], "general");
        let message = file_message(self_id, "report.pdf", 10);

        let caption = attachment_caption(&app, &message).unwrap();

        assert!(caption.contains("report.pdf"));
        assert!(!caption.contains("/save"), "got: {caption}");
    }

    #[test]
    fn attachment_caption_is_none_for_a_plain_text_message() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = ChatMessage {
            v: 2,
            id: 1,
            sender: [2; 32],
            ts_unix_ms: 0,
            text: "hello".to_string(),
            attachment: None,
        };

        assert_eq!(attachment_caption(&app, &message), None);
    }
}
