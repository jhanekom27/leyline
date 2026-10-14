//! Rendering for the `/files` overlay.

use super::*;

/// (`nav_scroll_offset`).
pub(super) fn render_files(frame: &mut Frame, app: &AppState) {
    let Some(overlay) = &app.viewing_files else {
        return;
    };
    let files = app.files_matches();
    let highlighted = app.files_highlight();

    let area = files_area(frame.area());
    let block = Block::bordered()
        .title(files_title(
            &app.channel_heading(&overlay.channel),
            files.len(),
            app.channel_files().len(),
            area.width as usize,
        ))
        .title_bottom(files_footer(
            app,
            highlighted.map(|position| &files[position]),
            overlay.notice,
        ))
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    // Cleared first, so nothing from the panes underneath shows through
    // the blank cells of the box.
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let [query_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    render_filter_row(frame, query_area, &overlay.query);

    if files.is_empty() {
        // With nothing typed, nothing matching means nothing shared.
        let text = if overlay.query.is_empty() {
            " no files have been shared in this channel yet"
        } else {
            " no matches"
        };
        let placeholder = Span::styled(
            text,
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        );
        frame.render_widget(Paragraph::new(Line::from(placeholder)), list_area);
        return;
    }
    let visible_rows = (list_area.height as usize).max(1);
    let start = nav_scroll_offset(files.len(), visible_rows, highlighted.unwrap_or(0));
    let now = Local::now();
    let items: Vec<ListItem> = files
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_rows)
        .map(|(row, shared)| {
            files_row(
                app,
                shared,
                app.saved_copy(shared.file.hash).is_some(),
                Some(row) == highlighted,
                list_area.width as usize,
                now,
            )
        })
        .collect();
    frame.render_widget(List::new(items), list_area);
    // As for the switcher, a track a row taller than the list at each end.
    let track = Rect::new(
        area.x,
        list_area.y.saturating_sub(1),
        area.width,
        list_area.height + 2,
    );
    render_scroll_indicator(frame, track, start, files.len(), visible_rows);
}

/// Where the files overlay box goes on a `screen`-sized frame: `FILES_WIDTH`
/// wide, with room for `FILES_ROWS` files whatever the channel holds -- see
/// `overlay_area`.
pub(super) fn files_area(screen: Rect) -> Rect {
    overlay_area(screen, FILES_WIDTH, FILES_ROWS)
}

/// The files overlay's title: the channel whose files it lists, by its
/// heading (`#general`, `@Alice`) -- cut short if it won't all fit -- and how
/// many files there are, `3 of 12` once the filter has narrowed them.
/// `width` is the box's, borders included.
pub(super) fn files_title(heading: &str, shown: usize, total: usize, width: usize) -> String {
    let lead = " files in ";
    let counts = if shown == total {
        format!(" · {total}")
    } else {
        format!(" · {shown} of {total}")
    };
    // Two cells of border, and a blank at the end (the lead has its own).
    let room = width.saturating_sub(2 + lead.chars().count() + counts.chars().count() + 1);
    format!("{lead}{}{counts} ", truncate_chars(heading, room))
}

/// What the files overlay's bottom border says: its `notice` -- why the last
/// key did nothing -- if it has one, in the picker's notice style, else the
/// keys that do something for the highlighted file (`AppState::file_offer`):
/// `Enter` saves one that isn't saved, `Ctrl+O` opens a saved one that can be
/// opened, and a saved program or script is offered neither -- so no key on
/// offer is a dead one.
pub(super) fn files_footer(
    app: &AppState,
    highlighted: Option<&SharedFile>,
    notice: Option<FilesNotice>,
) -> Line<'static> {
    if let Some(notice) = notice {
        let text = match notice {
            FilesNotice::AlreadySaved => " already saved · Ctrl+O opens it ",
            FilesNotice::NotSaved => " not saved yet · Enter saves it ",
            FilesNotice::SavedNotOpened => {
                " already saved · won't open programs or scripts from here "
            }
        };
        return Line::styled(
            text,
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    }
    Line::from(match highlighted {
        None => " Esc close ",
        Some(shared) => match app.file_offer(shared) {
            FileOffer::Save => " ↑/↓ move · Enter save · Esc close ",
            FileOffer::Open(_) => " ↑/↓ move · Ctrl+O open · Esc close ",
            FileOffer::SavedOnly => " ↑/↓ move · Esc close ",
        },
    })
}

/// One row of the files overlay, `width` cells wide: a gutter holding a check
/// mark if the file is `saved`, the name -- cut short with an ellipsis where
/// it would run into the columns on the right -- its size, who shared it
/// (`fit_name`, in their color) and when (`sent_label`, against `now`). A
/// `highlighted` row is shown black on cyan across its whole width, like the
/// picker's, so it carries no colors of its own.
pub(super) fn files_row(
    app: &AppState,
    shared: &SharedFile,
    saved: bool,
    highlighted: bool,
    width: usize,
    now: DateTime<Local>,
) -> ListItem<'static> {
    // The size, sender and date columns, two blanks between each, and a
    // blank at the right edge.
    let columns_width = PICKER_SIZE_WIDTH + 2 + NAME_WIDTH + 2 + PICKER_TIME_WIDTH + 1;
    let name_width = width.saturating_sub(PICKER_GUTTER + columns_width).max(1);
    let styled = |text: String, style: Style| {
        if highlighted {
            Span::raw(text)
        } else {
            Span::styled(text, style)
        }
    };
    let muted = Style::default().fg(Color::DarkGray);

    let gutter = if saved { " ✓ " } else { "   " };
    let spans = vec![
        styled(
            gutter.to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        styled(
            format!(
                "{:<name_width$}",
                truncate_chars(&shared.file.filename, name_width)
            ),
            Style::default(),
        ),
        styled(
            format!("{:>PICKER_SIZE_WIDTH$}  ", human_size(shared.file.size)),
            muted,
        ),
        styled(
            fit_name(&app.display_name(&shared.sender), NAME_WIDTH),
            Style::default().fg(user_color(&shared.sender)),
        ),
        styled(format!("  {}", sent_label(shared.ts_unix_ms, now)), muted),
    ];
    let item = ListItem::new(Line::from(spans));
    if highlighted {
        return item.style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    }
    item
}

/// When a file was shared, in `PICKER_TIME_WIDTH` cells: `day_label` of its
/// message's timestamp, which (message.rs) is Unix milliseconds in UTC. A
/// timestamp chrono can't represent -- practically unreachable -- shows as
/// dashes rather than panicking, as `format_time` does.
pub(super) fn sent_label(ts_unix_ms: u64, now: DateTime<Local>) -> String {
    match DateTime::from_timestamp_millis(ts_unix_ms as i64) {
        Some(sent) => day_label(sent.with_timezone(&Local), now),
        None => format!("{:<PICKER_TIME_WIDTH$}", "--"),
    }
}
