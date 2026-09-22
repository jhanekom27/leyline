//! Pure rendering: reads `AppState`, writes widgets to the frame. No
//! mutation, no I/O -- see concept.md's "one owner of UI state" principle.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, Paragraph};

use crate::app::{AppState, display_name};

pub fn render(frame: &mut Frame, app: &AppState) {
    let [header, body, input] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(3),
    ])
    .areas(frame.area());

    render_header(frame, header);
    render_body(frame, body, app);
    render_input(frame, input, app);
}

fn render_header(frame: &mut Frame, area: Rect) {
    let header = Paragraph::new(Line::from(vec![
        Span::styled(" leyline ", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("#general -- fake local data, no networking yet"),
    ]))
    .style(Style::default().bg(Color::DarkGray));
    frame.render_widget(header, area);
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
            let name = display_name(&m.sender);
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
        .map(|p| ListItem::new(format!("* {p}")))
        .collect();
    let list = List::new(items).block(Block::bordered().title("online"));
    frame.render_widget(list, area);
}

fn render_input(frame: &mut Frame, area: Rect, app: &AppState) {
    let input = Paragraph::new(format!("> {}", app.input))
        .block(Block::bordered().title("message (Enter to send, Esc to quit)"));
    frame.render_widget(input, area);
}
