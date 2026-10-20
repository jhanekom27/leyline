use super::*;

pub(super) fn render_peers(frame: &mut Frame, area: Rect, app: &AppState) {
    let items: Vec<ListItem> = peers_panel_rows(app)
        .iter()
        .map(|peer| peer_item(app, peer))
        .collect();

    let list = List::new(items).block(
        Block::bordered()
            .title("peers")
            .border_type(BorderType::Rounded),
    );
    frame.render_widget(list, area);
}

/// One peers-sidebar row. Online peers keep their colored dot; remembered
/// offline peers are dimmed -- when they were last seen is in their hover
/// popup (`render_peer_popup`) rather than crowding the row.
pub(super) fn peer_item(app: &AppState, peer: &PeerPresence) -> ListItem<'static> {
    match peer.last_seen_unix_ms {
        None => ListItem::new(Line::from(vec![
            Span::styled("● ", Style::default().fg(user_color(&peer.id))),
            Span::raw(app.display_name(&peer.id)),
        ])),
        Some(_) => ListItem::new(Line::from(vec![
            Span::styled("○ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                app.display_name(&peer.id),
                Style::default().fg(Color::DarkGray),
            ),
        ])),
    }
}

/// Renders the sidebar's command reference panel: one row per catalog entry,
/// showing just the bare usage, clipped to fit -- hovering a row shows its
/// whole usage and description (`render_hint_popup`), and full details
/// still live in `/help`'s system notice and the README.
pub(super) fn render_hints(frame: &mut Frame, area: Rect) {
    let items: Vec<ListItem> = command_catalog()
        .iter()
        .map(|command| ListItem::new(command.syntax))
        .collect();
    let list = List::new(items).block(
        Block::bordered()
            .title("commands")
            .border_type(BorderType::Rounded),
    );
    frame.render_widget(list, area);
}

/// Draws the hovered command hint's popup (`AppState::hovered_hint`): the
/// command's one-line description and, under it, its whole usage.
pub(super) fn render_hint_popup(frame: &mut Frame, app: &AppState) {
    let Some(index) = app.hovered_hint() else {
        return;
    };
    let Some(command) = command_catalog().get(index) else {
        return;
    };
    let (hints, inner) = hints_areas(frame.area(), app);
    let entries = vec![
        vec![Span::raw(command.description)],
        vec![Span::styled(
            command.syntax,
            Style::default().fg(Color::Cyan),
        )],
    ];
    render_popup(frame, hints, inner.y + index as u16, entries);
}

/// Draws the hovered peer's popup (`AppState::hovered_peer`), one entry per
/// line: our alias for them (only if we've set one), their nickname
/// followed by their short hex id in parentheses (or just the short id, if
/// they haven't announced one) -- alias and nickname in their identity
/// colour, like their dot -- then their online status and, only while
/// they're online, their announced build version.
pub(super) fn render_peer_popup(frame: &mut Frame, app: &AppState) {
    let Some(peer) = app.hovered_peer() else {
        return;
    };
    let (peers, inner) = peers_areas(frame.area(), app);
    let Some(index) = peers_panel_rows(app).iter().position(|p| p.id == peer.id) else {
        return;
    };
    let identity = Style::default().fg(user_color(&peer.id));
    let dim = Style::default().fg(Color::DarkGray);
    let mut entries = Vec::new();
    if let Some(alias) = peer.alias {
        entries.push(vec![Span::raw("Alias: "), Span::styled(alias, identity)]);
    }
    entries.push(match peer.nickname {
        Some(nickname) => vec![
            Span::styled(nickname, identity),
            Span::raw(format!(" ({})", hex_prefix(&peer.id))),
        ],
        None => vec![Span::styled(hex_prefix(&peer.id), identity)],
    });
    entries.push(match peer.last_seen_unix_ms {
        None => vec![Span::styled("Online", Style::default().fg(Color::Green))],
        Some(seen_unix_ms) => vec![Span::styled(
            format!("Offline: last seen {}", format_date_time(seen_unix_ms)),
            dim,
        )],
    });
    // A version is only worth showing for someone who's here now -- an
    // offline peer's may well be stale.
    if peer.last_seen_unix_ms.is_none() {
        entries.push(match peer.version {
            Some(version) => vec![Span::raw(format!("v{version}"))],
            None => vec![Span::styled("version unknown", dim)],
        });
    }
    render_popup(frame, peers, inner.y + index as u16, entries);
}

/// Draws a sidebar hover popup: a cleared, bordered box holding `entries`,
/// one per line. It opens to the left of `panel`, level with `row`, since
/// the sidebar sits against the right edge; the box is shifted up to stay
/// on screen, and an entry too wide for the terminal wraps (keeping each
/// span's style) rather than being clipped.
pub(super) fn render_popup(
    frame: &mut Frame,
    panel: Rect,
    row: u16,
    entries: Vec<Vec<Span<'static>>>,
) {
    let screen = frame.area();
    // One cell of padding either side of the text, inside the borders.
    let needed = entries
        .iter()
        .map(|spans| spans.iter().map(Span::width).sum::<usize>())
        .max()
        .unwrap_or(0)
        + 4;
    let width = (needed as u16).min(screen.width);
    let text_width = (width as usize).saturating_sub(4).max(1);
    let lines: Vec<Line> = entries
        .iter()
        .flat_map(|spans| wrap_spans(spans, text_width))
        .collect();
    let height = (lines.len() as u16 + 2).min(screen.height);
    let x = panel.x.saturating_sub(width);
    let y = row.min(screen.bottom().saturating_sub(height));
    let area = Rect::new(x, y, width, height);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Rounded)),
        area,
    );
}

/// Breaks `spans` into rows of at most `width` characters, like
/// `markdown::wrap_chars`, padding each row with a space either side and
/// keeping every character's original style.
pub(super) fn wrap_spans(spans: &[Span<'static>], width: usize) -> Vec<Line<'static>> {
    let chars: Vec<(char, Style)> = spans
        .iter()
        .flat_map(|span| span.content.chars().map(move |c| (c, span.style)))
        .collect();
    if chars.is_empty() {
        return vec![Line::from("")];
    }
    chars
        .chunks(width.max(1))
        .map(|chunk| {
            let mut text: Vec<Span<'static>> = Vec::new();
            for &(c, style) in chunk {
                match text.last_mut() {
                    Some(last) if last.style == style => last.content.to_mut().push(c),
                    _ => text.push(Span::styled(c.to_string(), style)),
                }
            }
            let mut row = vec![Span::raw(" ")];
            row.extend(text);
            row.push(Span::raw(" "));
            Line::from(row)
        })
        .collect()
}

pub(super) fn render_input(frame: &mut Frame, area: Rect, app: &AppState) {
    let prompt = format!("{} › ", app.display_name(&app.self_id));
    let title = match app.replying_to {
        Some(reply_to) => reply_banner_title(app, reply_to),
        // No reply is explicitly armed, but a plain send still
        // auto-joins the open thread (`AppState::active_thread_reply_target`)
        // -- say so, rather than leaving that implicit.
        None if app.active().thread.is_some() => {
            " replying within this thread · Esc to leave ".to_string()
        }
        None => " type a message · /help for commands ".to_string(),
    };
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let prompt_width = prompt.chars().count();
    let text_width = (inner.width as usize).saturating_sub(prompt_width).max(1);
    let visible_rows = (inner.height as usize).max(1);

    // Multi-line input never word-wraps a single logical line inside the
    // box (that would need mapping the cursor through wrapped text, which
    // `ui::markdown`-style rendering can afford since it's read-only, but
    // the input box also needs to place a live terminal cursor) -- each
    // `\n`-delimited line horizontal-scrolls independently instead,
    // exactly like the old single-line box did.
    let lines: Vec<&str> = app.active().input.split('\n').collect();
    let (cursor_line, cursor_col) = cursor_line_and_col(&app.active().input, app.active().cursor);

    // Vertically window `lines` so the cursor's line is always visible,
    // mirroring `render_messages`'s approach to keeping its reply-pick
    // highlight in view -- simpler here since the input box has no
    // persisted scroll state of its own to preserve between renders.
    let line_start = if cursor_line < visible_rows {
        0
    } else {
        cursor_line + 1 - visible_rows
    };
    let line_end = (line_start + visible_rows).min(lines.len());

    for (row_offset, line) in lines[line_start..line_end].iter().enumerate() {
        let line_index = line_start + row_offset;
        let chars: Vec<char> = line.chars().collect();

        let (visible_text, col_on_row) = if line_index == cursor_line {
            // Horizontal-scroll the focused line so the cursor always
            // stays in view.
            let visible_start = cursor_col.saturating_sub(text_width.saturating_sub(1));
            let visible_end = (visible_start + text_width).min(chars.len());
            (
                chars[visible_start..visible_end].iter().collect::<String>(),
                cursor_col - visible_start,
            )
        } else {
            // Other lines just clip from the start -- only the focused
            // line needs to track the cursor horizontally.
            let visible_end = text_width.min(chars.len());
            (chars[..visible_end].iter().collect::<String>(), 0)
        };

        // The colored prompt only ever prefixes the message's actual
        // first line; continuation lines get blank padding of the same
        // width so their text still lines up underneath it.
        let spans = if line_index == 0 {
            vec![
                Span::styled(
                    prompt.clone(),
                    Style::default()
                        .fg(user_color(&app.self_id))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(visible_text),
            ]
        } else {
            vec![Span::raw(" ".repeat(prompt_width)), Span::raw(visible_text)]
        };

        let row_y = inner.y + row_offset as u16;
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(inner.x, row_y, inner.width, 1),
        );
        if line_index == cursor_line {
            let cursor_x = inner.x + (prompt_width + col_on_row) as u16;
            frame.set_cursor_position((cursor_x, row_y));
        }
    }
}

/// The strip of attachments staged on the active channel
/// (`Channel::staged`), drawn just above the input box so what is about to
/// go out -- and how to take it back -- is in plain sight before `Enter`.
/// Draws nothing when nothing is staged (`staged_strip_height` gave it no
/// room). The key hints live in the border title, like the other panels'.
pub(super) fn render_staged(frame: &mut Frame, area: Rect, app: &AppState) {
    let staged = &app.active().staged;
    if staged.is_empty() {
        return;
    }
    let block = Block::bordered()
        .title(" staged · Enter send · Backspace remove last · Esc clear ")
        .border_type(BorderType::Rounded);
    let rows = staged_rows(
        staged,
        block.inner(area).width as usize,
        user_color(&app.self_id),
    );
    frame.render_widget(Paragraph::new(rows).block(block), area);
}

/// One row per staged attachment, oldest first, so the newest sits nearest
/// the input box. When they don't all fit (`MAX_STAGED_ROWS`), the oldest
/// collapse into one "and N more" row on top: the newest -- the one
/// `Backspace` takes back next -- always stays visible.
pub(super) fn staged_rows(
    staged: &[StagedAttachment],
    width: usize,
    marker: Color,
) -> Vec<Line<'static>> {
    let (hidden, shown) = if staged.len() > MAX_STAGED_ROWS {
        let hidden = staged.len() - (MAX_STAGED_ROWS - 1);
        (hidden, &staged[hidden..])
    } else {
        (0, staged)
    };
    let mut rows = Vec::with_capacity(MAX_STAGED_ROWS);
    if hidden > 0 {
        rows.push(Line::from(Span::styled(
            format!("   … and {hidden} more"),
            Style::default().fg(Color::DarkGray),
        )));
    }
    rows.extend(shown.iter().map(|item| staged_row(item, width, marker)));
    rows
}

/// A staged attachment's row: a marker, its name (truncated with an
/// ellipsis if it would overflow `width`, so the size beside it always
/// stays visible), and its size.
pub(super) fn staged_row(item: &StagedAttachment, width: usize, marker: Color) -> Line<'static> {
    let size = human_size(item.size);
    // " + " before the name and two spaces before the size.
    let name_width = width.saturating_sub(3 + 2 + size.chars().count()).max(1);
    Line::from(vec![
        Span::styled(" + ", Style::default().fg(marker)),
        Span::styled(
            truncate_chars(&item.filename, name_width),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {size}"), Style::default().fg(Color::DarkGray)),
    ])
}

/// Maps a flat char-index cursor position into (line index, column within
/// that line) against `input`'s `\n`-delimited lines -- used by
/// `render_input` to decide which logical line's horizontal scroll
/// window to compute and where to place the terminal cursor.
pub(super) fn cursor_line_and_col(input: &str, cursor: usize) -> (usize, usize) {
    let mut remaining = cursor;
    let mut last_index = 0;
    let mut last_len = 0;
    for (index, line) in input.split('\n').enumerate() {
        let len = line.chars().count();
        if remaining <= len {
            return (index, remaining);
        }
        remaining -= len + 1; // +1 for the '\n' consumed between lines
        last_index = index;
        last_len = len;
    }
    (last_index, last_len)
}

/// Builds the input box's title while a reply is armed
/// (`AppState::replying_to`): the parent's sender and a truncated snippet
/// (see `reply_snippet`), so it's obvious what you're replying to for as
/// long as it stays armed. Mirrors `reply_preview_spans`'s fallback if the
/// parent isn't currently resolvable.
pub(super) fn reply_banner_title(app: &AppState, reply_to: u64) -> String {
    match app.active().find_message(reply_to) {
        Some(parent) => format!(
            " replying to {}: {} · Esc to cancel ",
            app.display_name(&parent.sender),
            reply_snippet(parent)
        ),
        None => " replying to a message that's no longer available · Esc to cancel ".to_string(),
    }
}
