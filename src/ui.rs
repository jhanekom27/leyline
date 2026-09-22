//! Pure rendering: reads `AppState`, writes widgets to the frame. No
//! mutation, no I/O -- see concept.md's "one owner of UI state" principle.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};

use crate::app::{AppState, hex_id};

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
    let [title, identity] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(area);

    let title_line = Paragraph::new(Line::from(vec![
        Span::styled(" leyline ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "#general -- {} peer(s) online",
            app.peers.len()
        )),
    ]))
    .style(Style::default().bg(Color::DarkGray));
    frame.render_widget(title_line, title);

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
    let visible = area.height.saturating_sub(2).max(1) as usize; // minus borders
    let total = app.messages.len();
    let end = total.saturating_sub(app.scroll);
    let start = end.saturating_sub(visible);

    let items: Vec<ListItem> = app
        .messages
        .iter()
        .skip(start)
        .take(end - start)
        .map(|m| {
            let name = app.display_name(&m.sender);
            ListItem::new(Line::from(vec![
                Span::styled(format!("{name}: "), Style::default().fg(Color::Cyan)),
                Span::raw(m.text.clone()),
            ]))
        })
        .collect();

    let list = List::new(items).block(Block::bordered().title("messages"));
    frame.render_widget(list, area);
}

fn render_peers(frame: &mut Frame, area: Rect, app: &AppState) {
    let items: Vec<ListItem> = app
        .peers
        .iter()
        .map(|p| ListItem::new(format!("* {}", app.display_name(p))))
        .collect();
    let list = List::new(items).block(Block::bordered().title("online"));
    frame.render_widget(list, area);
}

fn render_input(frame: &mut Frame, area: Rect, app: &AppState) {
    const PROMPT: &str = "> ";
    let block = Block::bordered().title("message (Enter to send, Esc to quit)");
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
