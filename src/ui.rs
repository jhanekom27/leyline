//! Pure rendering: reads `AppState`, writes widgets to the frame. No
//! mutation, no I/O -- see concept.md's "one owner of UI state" principle.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};
use textwrap::wrap;

use crate::app::{AppState, Channel, TranscriptLine, hex_id};

/// Width of the `HH:MM` time column in a chat row.
const TIME_WIDTH: usize = 5;
/// Width of the sender-name column in a chat row; longer names are
/// truncated with an ellipsis so the `│` separator after it always lines
/// up (see `fit_name`).
const NAME_WIDTH: usize = 10;
/// Separates the name column from the message text.
const SEPARATOR: &str = " │ ";

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
    let [messages, peers] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(22)]).areas(area);

    render_messages(frame, messages, app);
    render_peers(frame, peers, app);
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
    let end = total.saturating_sub(channel.scroll);
    let start = end.saturating_sub(visible_rows);
    let items: Vec<ListItem> = rows.drain(start..end).map(ListItem::new).collect();

    let title = format!("#{} · {} peer(s)", channel.name, channel.peers.len());
    let list = List::new(items).block(Block::bordered().title(title));
    frame.render_widget(list, area);
}

/// Renders `channel`'s transcript into rendered rows: a colored rail, time,
/// sender name, separator, and message text for `Chat` lines (see the
/// module-level column layout), or the existing dim/italic notice style for
/// `System` lines. Sender grouping -- blanking the time/name (rail stays)
/// on a run of messages from the same person, or on a message's own
/// word-wrapped continuation lines -- is computed here since it depends on
/// iterating the transcript in order.
fn build_message_rows(app: &AppState, channel: &Channel, inner_width: usize) -> Vec<Line<'static>> {
    let prefix_width = 1 + 1 + TIME_WIDTH + 1 + NAME_WIDTH + SEPARATOR.chars().count();
    let wrap_width = inner_width.saturating_sub(prefix_width).max(10);
    let blank_time = " ".repeat(TIME_WIDTH);
    let blank_name = " ".repeat(NAME_WIDTH);

    let mut rows = Vec::new();
    let mut last_chat_sender: Option<[u8; 32]> = None;
    for line in &channel.messages {
        match line {
            TranscriptLine::Chat(message) => {
                let is_first_of_group = last_chat_sender != Some(message.sender);
                last_chat_sender = Some(message.sender);

                let color = user_color(&message.sender);
                let name = fit_name(&app.display_name(&message.sender), NAME_WIDTH);
                let time = format_time(message.ts_unix_ms);

                let mut wrapped = wrap(&message.text, wrap_width);
                if wrapped.is_empty() {
                    wrapped.push(std::borrow::Cow::Borrowed(""));
                }
                for (index, chunk) in wrapped.iter().enumerate() {
                    let show_header = index == 0 && is_first_of_group;
                    let (time_span, name_span) = if show_header {
                        (time.clone(), name.clone())
                    } else {
                        (blank_time.clone(), blank_name.clone())
                    };
                    rows.push(Line::from(vec![
                        Span::styled(" ", Style::default().bg(color)),
                        Span::raw(" "),
                        Span::styled(time_span, Style::default().fg(Color::DarkGray)),
                        Span::raw(" "),
                        Span::styled(
                            name_span,
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(SEPARATOR, Style::default().fg(Color::DarkGray)),
                        Span::raw(chunk.to_string()),
                    ]));
                }
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

    let list = List::new(items).block(Block::bordered().title("peers"));
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

fn render_input(frame: &mut Frame, area: Rect, app: &AppState) {
    let prompt = format!("{} › ", app.display_name(&app.self_id));
    let block = Block::bordered().title(" type a message · /help for commands ");
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
