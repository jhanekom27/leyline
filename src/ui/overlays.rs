use super::*;

/// Splits a command-picker row into command, action, shortcut, and
/// description columns. Action and shortcut columns stay fixed where space
/// permits so their empty or abbreviated states cannot be mistaken for text
/// from a neighbouring cell.
pub(super) fn command_picker_columns(width: usize) -> (usize, usize, usize, usize) {
    let content_width = width.saturating_sub(1);
    let action_width = content_width.min(COMMAND_PICKER_ACTION_WIDTH);
    let shortcut_width = content_width.min(COMMAND_PICKER_SHORTCUT_WIDTH);
    let remaining = content_width.saturating_sub(action_width + shortcut_width);
    let syntax_width = remaining / 2;
    let description_width = remaining.saturating_sub(syntax_width);
    (
        syntax_width,
        action_width,
        shortcut_width,
        description_width,
    )
}

pub(super) fn command_picker_header(width: usize) -> Line<'static> {
    let (syntax_width, action_width, shortcut_width, description_width) =
        command_picker_columns(width);
    Line::from(vec![
        Span::styled(
            format!(" {:<syntax_width$}", "command"),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<action_width$}", "action"),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<shortcut_width$}", "shortcut"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            truncate_chars("description", description_width),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

/// One command-picker row, with every span constrained to a dedicated
/// column so narrow terminals stay legible.
pub(super) fn command_picker_row(
    command: &CommandDefinition,
    width: usize,
    highlighted: bool,
) -> ListItem<'static> {
    let (syntax_width, action_width, shortcut_width, description_width) =
        command_picker_columns(width);
    let syntax_style = if highlighted {
        Style::default().fg(Color::Black)
    } else {
        Style::default().fg(Color::Cyan)
    };
    let shortcut_style = if highlighted {
        Style::default().fg(Color::Black)
    } else {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    };
    let description_style = if highlighted {
        Style::default().fg(Color::Black)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let action_style = if highlighted {
        Style::default().fg(Color::Black)
    } else {
        match command.picker_action {
            CommandPickerAction::Execute => Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
            CommandPickerAction::Insert => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        }
    };
    let action = match command.picker_action {
        CommandPickerAction::Execute => "▶ run",
        CommandPickerAction::Insert => "✎ edit",
    };
    let mut spans = vec![Span::styled(
        format!(
            " {:<syntax_width$}",
            truncate_chars(command.syntax, syntax_width)
        ),
        syntax_style,
    )];
    spans.push(Span::styled(
        format!("{:<action_width$}", truncate_chars(action, action_width)),
        action_style,
    ));
    spans.push(Span::styled(
        format!(
            "{:<shortcut_width$}",
            truncate_chars(command.shortcut.unwrap_or_default(), shortcut_width)
        ),
        shortcut_style,
    ));
    spans.push(Span::styled(
        truncate_chars(command.description, description_width),
        description_style,
    ));
    let item = ListItem::new(Line::from(spans));
    if highlighted {
        item.style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        item
    }
}

pub(super) fn command_picker_area(screen: Rect, matches: usize) -> Rect {
    overlay_area(
        screen,
        COMMAND_PICKER_WIDTH,
        matches.clamp(1, COMMAND_PICKER_ROWS) + 1,
    )
}

/// Where the switcher box goes on a `screen`-sized frame when `matches`
/// channels are listed: `SWITCHER_WIDTH` wide, tall enough for up to
/// `SWITCHER_MAX_ROWS` matches (one row even with none, for the
/// placeholder), and shrinking as typing narrows the list -- see
/// `overlay_area`.
pub(super) fn switcher_area(screen: Rect, matches: usize) -> Rect {
    overlay_area(screen, SWITCHER_WIDTH, matches.clamp(1, SWITCHER_MAX_ROWS))
}
/// The slash-command picker opened with `Ctrl+K`. It mirrors the switcher's
/// modal behavior, but rows show each command's syntax, action, shortcut, if
/// any, and description in dedicated columns.
pub(super) fn render_command_picker(frame: &mut Frame, app: &AppState) {
    let Some(picker) = &app.command_picker else {
        return;
    };
    let matches = app.command_picker_matches();
    let highlighted = app.command_picker_highlight();
    let area = command_picker_area(frame.area(), matches.len());
    let instruction = highlighted
        .map(|index| match command_catalog()[index].picker_action {
            CommandPickerAction::Execute => "Enter run · draft kept",
            CommandPickerAction::Insert => "Enter insert · replaces draft",
        })
        .unwrap_or("Enter select");
    let block = Block::bordered()
        .title("commands · Ctrl+K")
        .title_bottom(format!(
            "↑/↓ move · {instruction} · Ctrl+P channels · Esc cancel"
        ))
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let [query_area, header_area, list_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);
    render_filter_row(frame, query_area, &picker.query);
    frame.render_widget(
        Paragraph::new(command_picker_header(header_area.width as usize)),
        header_area,
    );

    if matches.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                " no matching commands",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
            list_area,
        );
        return;
    }

    let visible_rows = (list_area.height as usize).max(1);
    let highlighted_row = matches
        .iter()
        .position(|&index| Some(index) == highlighted)
        .unwrap_or(0);
    let start = nav_scroll_offset(matches.len(), visible_rows, highlighted_row);
    let items = matches
        .iter()
        .skip(start)
        .take(visible_rows)
        .map(|&index| {
            command_picker_row(
                &command_catalog()[index],
                list_area.width as usize,
                Some(index) == highlighted,
            )
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(items), list_area);
    let track = Rect::new(
        area.x,
        list_area.y.saturating_sub(1),
        area.width,
        list_area.height + 2,
    );
    render_scroll_indicator(frame, track, start, matches.len(), visible_rows);
}

pub(super) fn peers_overlay_area(screen: Rect, peers: usize) -> Rect {
    overlay_area(screen, PEERS_OVERLAY_WIDTH, peers.max(1) + 1)
}

pub(super) fn peers_overlay_columns(width: usize) -> (usize, usize, usize) {
    let content_width = width.saturating_sub(1);
    let details_width = content_width.saturating_sub(70).min(28);
    let endpoint_width = content_width.saturating_sub(details_width);
    let alias_width = details_width / 2;
    let nickname_width = details_width.saturating_sub(alias_width);
    (endpoint_width, alias_width, nickname_width)
}

pub(super) fn peers_overlay_header(width: usize) -> Line<'static> {
    let (endpoint_width, alias_width, nickname_width) = peers_overlay_columns(width);
    Line::from(vec![
        Span::styled(
            format!(
                " {:<endpoint_width$}",
                truncate_chars("      endpoint id", endpoint_width)
            ),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<alias_width$}", truncate_chars("alias", alias_width)),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            truncate_chars("nickname", nickname_width),
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

pub(super) fn peers_overlay_row(
    peer: &PeerDetails,
    self_id: [u8; 32],
    width: usize,
) -> ListItem<'static> {
    let (endpoint_width, alias_width, nickname_width) = peers_overlay_columns(width);
    let endpoint_id_width = endpoint_width.saturating_sub(6);
    let alias = peer.alias.as_deref().unwrap_or("—");
    let nickname = peer.nickname.as_deref().unwrap_or("—");
    let (presence, presence_style, identity_style) = if peer.id == self_id {
        (
            "● you ",
            Style::default().fg(Color::Green),
            Style::default().fg(user_color(&peer.id)),
        )
    } else if peer.is_ours {
        match peer.last_seen_unix_ms {
            None => (
                "● mine",
                Style::default().fg(Color::Green),
                Style::default().fg(user_color(&peer.id)),
            ),
            Some(_) => (
                "○ mine",
                Style::default().fg(Color::DarkGray),
                Style::default().fg(Color::DarkGray),
            ),
        }
    } else {
        match peer.last_seen_unix_ms {
            None => (
                "●     ",
                Style::default().fg(user_color(&peer.id)),
                Style::default().fg(user_color(&peer.id)),
            ),
            Some(_) => (
                "○     ",
                Style::default().fg(Color::DarkGray),
                Style::default().fg(Color::DarkGray),
            ),
        }
    };
    ListItem::new(Line::from(vec![
        Span::raw(" "),
        Span::styled(presence, presence_style),
        Span::styled(
            format!(
                "{:<endpoint_id_width$}",
                truncate_chars(&hex_id(&peer.id), endpoint_id_width)
            ),
            identity_style,
        ),
        Span::styled(
            format!("{:<alias_width$}", truncate_chars(alias, alias_width)),
            Style::default().fg(Color::Yellow),
        ),
        Span::styled(
            truncate_chars(nickname, nickname_width),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
}

/// Renders the `/who` snapshot as a cleared, top-anchored table. The state
/// lives in `AppState`, so this renderer only reads it and leaves dismissal
/// to `AppState::handle_peers_overlay_key`.
pub(super) fn render_peers_overlay(frame: &mut Frame, app: &AppState) {
    let Some(overlay) = &app.viewing_peers else {
        return;
    };
    let area = peers_overlay_area(frame.area(), overlay.peers.len());
    let block = Block::bordered()
        .title(format!(" peers in {} ", overlay.heading))
        .title_bottom(" Esc close ")
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let [header_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    frame.render_widget(
        Paragraph::new(peers_overlay_header(header_area.width as usize)),
        header_area,
    );
    if overlay.peers.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::styled(
                " no peers currently in this channel",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
            list_area,
        );
        return;
    }
    let rows = overlay
        .peers
        .iter()
        .map(|peer| peers_overlay_row(peer, app.self_id, list_area.width as usize))
        .collect::<Vec<_>>();
    frame.render_widget(List::new(rows), list_area);
}
/// Where an overlay `width` wide, with room for `list_rows` rows under its
/// borders and its filter row, goes on a `screen`-sized frame: centered, and
/// anchored near the top so the box doesn't jump around as typing changes
/// how many rows it needs. Never larger than the screen. Shared by the
/// switcher and the file picker, which are laid out the same way.
pub(super) fn overlay_area(screen: Rect, width: u16, list_rows: usize) -> Rect {
    let width = width.min(screen.width);
    let height = (list_rows as u16 + 3).min(screen.height);
    let x = screen.x + (screen.width - width) / 2;
    let y = (screen.y + 2).min(screen.bottom() - height);
    Rect::new(x, y, width, height)
}
