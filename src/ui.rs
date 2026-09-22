//! Pure rendering: reads `AppState`, writes widgets to the frame. No
//! mutation, no I/O -- see concept.md's "one owner of UI state" principle.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};

use crate::app::{AppState, TranscriptLine, hex_id};

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
    spans.push(Span::raw(format!(
        " -- {} peer(s) online",
        app.active().peers.len()
    )));

    let tabs_line = Paragraph::new(Line::from(spans)).style(Style::default().bg(Color::DarkGray));
    frame.render_widget(tabs_line, tabs);

    let identity_line = Paragraph::new(Line::from(vec![
        Span::raw(" you: "),
        Span::styled(hex_id(&app.self_id), Style::default().fg(Color::Cyan)),
    ]));
    frame.render_widget(identity_line, identity);
}

fn render_body(frame: &mut Frame, area: Rect, app: &AppState) {
    let [messages, peers] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(16)]).areas(area);

    render_messages(frame, messages, app);
    render_peers(frame, peers, app);
}

fn render_messages(frame: &mut Frame, area: Rect, app: &AppState) {
    let channel = app.active();
    let visible = area.height.saturating_sub(2).max(1) as usize; // minus borders
    let total = channel.messages.len();
    let end = total.saturating_sub(channel.scroll);
    let start = end.saturating_sub(visible);
    let inner_width = (area.width as usize).saturating_sub(2).max(1); // minus borders

    let items: Vec<ListItem> = channel
        .messages
        .iter()
        .skip(start)
        .take(end - start)
        .map(|line| match line {
            TranscriptLine::Chat(m) => {
                let name = app.display_name(&m.sender);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{name}: "), Style::default().fg(Color::Cyan)),
                    Span::raw(m.text.clone()),
                ]))
            }
            TranscriptLine::System(text) => {
                // System lines can be long (e.g. invite tickets, which are
                // one unbroken token with no spaces to wrap on), and
                // `List` doesn't wrap on its own -- without this, anything
                // wider than the pane would just get clipped and
                // unreadable/uncopyable.
                let style = Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC);
                let lines: Vec<Line> = wrap_chars(&format!("* {text}"), inner_width)
                    .into_iter()
                    .map(|chunk| Line::from(Span::styled(chunk, style)))
                    .collect();
                ListItem::new(lines)
            }
        })
        .collect();

    let list = List::new(items).block(Block::bordered().title(format!("#{}", channel.name)));
    frame.render_widget(list, area);
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

fn render_peers(frame: &mut Frame, area: Rect, app: &AppState) {
    let channel = app.active();
    let items: Vec<ListItem> = channel
        .peers
        .iter()
        .map(|p| ListItem::new(format!("* {}", app.display_name(p))))
        .collect();
    let list = List::new(items).block(Block::bordered().title("online"));
    frame.render_widget(list, area);
}

fn render_input(frame: &mut Frame, area: Rect, app: &AppState) {
    const PROMPT: &str = "> ";
    let block = Block::bordered()
        .title("message (Enter to send, /join <name|ticket>, /invite, Tab to switch, Esc to quit)");
    let inner = block.inner(area);
    let text_width = (inner.width as usize).saturating_sub(PROMPT.len()).max(1);

    // Horizontal-scroll the input so the cursor always stays in view.
    let chars: Vec<char> = app.input.chars().collect();
    let visible_start = app.cursor.saturating_sub(text_width.saturating_sub(1));
    let visible_end = (visible_start + text_width).min(chars.len());
    let visible: String = chars[visible_start..visible_end].iter().collect();

    let paragraph = Paragraph::new(format!("{PROMPT}{visible}")).block(block);
    frame.render_widget(paragraph, area);

    let cursor_col = inner.x + (PROMPT.len() + (app.cursor - visible_start)) as u16;
    frame.set_cursor_position((cursor_col, inner.y));
}
