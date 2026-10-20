use super::*;

/// The one-row strip above the body: who you are (your full endpoint id,
/// which a peer can paste into `/dm` to reach you directly) and, once a
/// peer has been seen running a newer build, a standing update notice. The
/// channel list lives in the navigation panel (`render_nav`).
pub(super) fn render_header(frame: &mut Frame, area: Rect, app: &AppState) {
    let mut identity_spans = vec![
        Span::raw(" you: "),
        Span::styled(
            hex_id(&app.self_id),
            Style::default().fg(user_color(&app.self_id)),
        ),
    ];
    // Durable for the whole session once seen (unlike the one-time system
    // notice `AppState::handle_net_event` pushes on first detection), so it
    // doesn't just scroll out of the transcript -- see
    // `AppState::newest_known_update` and features.md's version-
    // notification idea.
    if let Some((version, _)) = app.newest_known_update() {
        identity_spans.push(Span::styled(
            format!("   update available: v{version}"),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    let identity_line = Paragraph::new(Line::from(identity_spans));
    frame.render_widget(identity_line, area);
}

pub(super) fn render_body(frame: &mut Frame, area: Rect, app: &AppState) {
    let [nav, messages, sidebar] = body_areas(area);

    render_nav(frame, nav, app);
    render_messages(frame, messages, app);
    render_sidebar(frame, sidebar, app);
}

/// The left-hand navigation panel: every joined channel on its own row in
/// one of two sections -- CHANNELS for group channels, PEOPLE for `/msg`
/// DMs and `/dm` point-to-point chats (`AppState::nav_sections`) -- led by
/// its kind's glyph (`ChannelKind::glyph`) and named by its current label
/// (`AppState::channel_label`). The active channel's row is highlighted
/// across the panel's width, and any other channel with unseen activity
/// ends in its unread count. Both section headers are always drawn, even over an
/// empty section, so the panel's shape doesn't jump as conversations come
/// and go.
///
/// When the rows outnumber the panel's height it scrolls to keep the
/// active row in view (`nav_scroll_offset`), with a thumb on its right
/// border (`render_scroll_indicator`).
pub(super) fn render_nav(frame: &mut Frame, area: Rect, app: &AppState) {
    let block = Block::bordered()
        .title("leyline")
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    let visible_rows = (inner.height as usize).max(1);

    let sections = app.nav_sections();
    let mut rows = Vec::new();
    let mut active_row = 0;
    for (title, members) in [
        ("CHANNELS", &sections.channels),
        ("PEOPLE", &sections.people),
    ] {
        rows.push(nav_section_header(title));
        for &index in members {
            if index == app.active {
                active_row = rows.len();
            }
            rows.push(nav_entry(
                app,
                index,
                inner.width as usize,
                index == app.active,
            ));
        }
    }

    let total = rows.len();
    let start = nav_scroll_offset(total, visible_rows, active_row);
    let items: Vec<ListItem> = rows.into_iter().skip(start).take(visible_rows).collect();
    frame.render_widget(List::new(items).block(block), area);
    render_scroll_indicator(frame, area, start, total, visible_rows);
}

/// A section title row (CHANNELS / PEOPLE), muted so the entries beneath
/// it stand out.
pub(super) fn nav_section_header(title: &str) -> ListItem<'static> {
    ListItem::new(Line::from(Span::styled(
        format!(" {title}"),
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )))
}

/// One channel's row, `width` cells wide: a space, the kind glyph padded
/// to `NAV_GLYPH_WIDTH` cells (measured rather than assumed, since the
/// point-to-point bolt is double-width), the label truncated to leave
/// enough cells for its right-aligned status or unread count. A
/// `highlighted` row -- the active channel in the navigation panel, the
/// selected match in the switcher -- is shown black on cyan across the
/// whole row and carries no marker: it's the channel you're on, or about
/// to be.
pub(super) fn nav_entry(
    app: &AppState,
    index: usize,
    width: usize,
    highlighted: bool,
) -> ListItem<'static> {
    let channel = &app.channels[index];

    let glyph = Span::raw(app.channel_kind(&channel.name).glyph());
    let padding = " ".repeat(NAV_GLYPH_WIDTH.saturating_sub(glyph.width()) + 1);
    let marker = (!highlighted)
        .then(|| {
            app.sync_marker(&channel.name)
                .map(|marker| marker.to_string())
                .or_else(|| (channel.unread_count > 0).then(|| channel.unread_count.to_string()))
        })
        .flatten();
    let marker_width = marker
        .as_ref()
        .map_or(0, |marker| marker.chars().count() + 1);
    let label = truncate_chars(
        &app.channel_label(&channel.name),
        width.saturating_sub(NAV_LABEL_OFFSET + marker_width),
    );
    let mut line = Line::from(vec![
        Span::raw(" "),
        glyph,
        Span::raw(padding),
        Span::raw(label),
    ]);

    if highlighted {
        return ListItem::new(line).style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }
    if let Some(marker) = marker {
        let gap = width.saturating_sub(line.width() + marker.chars().count());
        line.push_span(Span::raw(" ".repeat(gap)));
        line.push_span(Span::styled(
            marker,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
    }
    ListItem::new(line)
}

/// Index of the first of `total` navigation rows to draw in a window of
/// `visible` rows, chosen so `active_row` stays in view: centered where
/// there's room, held against the top or bottom otherwise. Everything
/// starts at row 0 while it all fits.
pub(super) fn nav_scroll_offset(total: usize, visible: usize, active_row: usize) -> usize {
    if total <= visible {
        return 0;
    }
    active_row.saturating_sub(visible / 2).min(total - visible)
}

/// The jump-to overlay opened with `Ctrl+P` (`AppState::switching`), drawn
/// over everything else: a cleared, bordered box holding the typed filter
/// and, under it, every channel that matches (`AppState::switcher_matches`)
/// in the navigation panel's own row format (`nav_entry`), with the
/// highlighted match in the active-channel style. Like the panel it scrolls
/// to keep the highlight in view once there are more matches than rows
/// (`nav_scroll_offset`).
pub(super) fn render_switcher(frame: &mut Frame, app: &AppState) {
    let Some(switcher) = &app.switching else {
        return;
    };
    let matches = app.switcher_matches();
    let highlighted = app.switcher_highlight();

    let area = switcher_area(frame.area(), matches.len());
    let block = Block::bordered()
        .title("jump to")
        .title_bottom("↑/↓ move · Enter confirm · Esc cancel")
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    // Cleared first, so nothing from the panes underneath shows through
    // the blank cells of the box.
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let [query_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);

    render_filter_row(frame, query_area, &switcher.query);

    if matches.is_empty() {
        let placeholder = Span::styled(
            " no matches",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        );
        frame.render_widget(Paragraph::new(Line::from(placeholder)), list_area);
        return;
    }
    let visible_rows = (list_area.height as usize).max(1);
    let highlighted_row = matches
        .iter()
        .position(|&index| Some(index) == highlighted)
        .unwrap_or(0);
    let start = nav_scroll_offset(matches.len(), visible_rows, highlighted_row);
    let items: Vec<ListItem> = matches
        .iter()
        .skip(start)
        .take(visible_rows)
        .map(|&index| {
            nav_entry(
                app,
                index,
                list_area.width as usize,
                Some(index) == highlighted,
            )
        })
        .collect();
    frame.render_widget(List::new(items), list_area);
    // The thumb belongs beside the list rows, not the query row above
    // them, so hand the indicator a track one row taller than the list at
    // each end -- it insets its own area by a row, as for a bordered block.
    let track = Rect::new(
        area.x,
        list_area.y.saturating_sub(1),
        area.width,
        list_area.height + 2,
    );
    render_scroll_indicator(frame, track, start, matches.len(), visible_rows);
}
