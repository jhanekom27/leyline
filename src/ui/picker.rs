use super::*;

/// An overlay's filter row: a prompt and the tail of what's been typed --
/// the end is what's being edited -- with the terminal cursor just after
/// it.
pub(super) fn render_filter_row(frame: &mut Frame, area: Rect, query: &str) {
    let prompt = "› ";
    let room = (area.width as usize).saturating_sub(Span::raw(prompt).width() + 1);
    let cut = query.chars().count().saturating_sub(room);
    let typed: String = query.chars().skip(cut).collect();
    let line = Line::from(vec![
        Span::styled(
            prompt,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(typed),
    ]);
    frame.set_cursor_position((area.x + line.width() as u16, area.y));
    frame.render_widget(Paragraph::new(line), area);
}

/// The file picker opened by a bare `/send` (`AppState::picking_file`),
/// drawn over everything else like the switcher: a cleared, bordered box
/// titled with the folder being browsed, holding the typed filter and, under
/// it, the folder's entries -- folders first, then files newest first --
/// with the highlighted row in the switcher's style. Its size is fixed, so
/// the box doesn't jump around as typing narrows the folder; it scrolls to
/// keep the highlight in view (`nav_scroll_offset`).
pub(super) fn render_picker(frame: &mut Frame, app: &AppState) {
    let Some(picker) = &app.picking_file else {
        return;
    };
    let area = picker_area(frame.area());
    let block = Block::bordered()
        .title(picker_title(picker, area.width as usize))
        .title_bottom(picker_footer(picker, area.width as usize))
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    // Cleared first, so nothing from the panes underneath shows through
    // the blank cells of the box.
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    let [query_area, list_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(inner);
    render_filter_row(frame, query_area, picker.query());

    let total = picker.visible().len();
    if total == 0 {
        let text = match (picker.is_empty(), picker.is_reading()) {
            (true, true) => " reading...",
            (true, false) => " empty folder",
            (false, _) => " no matches",
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
    let start = nav_scroll_offset(total, visible_rows, picker.selected());
    let now = Local::now();
    let items: Vec<ListItem> = picker
        .visible()
        .enumerate()
        .skip(start)
        .take(visible_rows)
        .map(|(row, entry)| {
            picker_row(
                entry,
                picker.is_marked(entry),
                row == picker.selected(),
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
    render_scroll_indicator(frame, track, start, total, visible_rows);
}

/// Where the file picker box goes on a `screen`-sized frame: `PICKER_WIDTH`
/// wide, with room for `PICKER_ROWS` entries whatever the folder holds --
/// see `overlay_area`.
pub(super) fn picker_area(screen: Rect) -> Rect {
    overlay_area(screen, PICKER_WIDTH, PICKER_ROWS)
}

/// The picker's title: the folder being browsed -- its tail, if it won't all
/// fit -- then how many files are marked and whether a folder is still being
/// read. `width` is the box's, borders included.
pub(super) fn picker_title(picker: &FilePicker, width: usize) -> String {
    let mut extras = String::new();
    if picker.marked_count() > 0 {
        extras.push_str(&format!(" · {} marked", picker.marked_count()));
    }
    if picker.is_reading() {
        extras.push_str(" · reading...");
    }
    // Two cells of border, and a blank either side of the title.
    let room = width.saturating_sub(2 + 2 + extras.chars().count());
    format!(
        " {}{extras} ",
        truncate_start(&picker.dir().to_string_lossy(), room)
    )
}

/// What the picker's bottom border says: its notice -- an unreadable
/// folder, say -- if it has one, else the keys. A notice too long for the
/// row loses its start, not its end: the reason comes last. `width` is the
/// box's, borders included.
pub(super) fn picker_footer(picker: &FilePicker, width: usize) -> Line<'static> {
    match picker.notice() {
        Some(notice) => Line::styled(
            format!(" {} ", truncate_start(notice, width.saturating_sub(4))),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        None => {
            Line::from(" ↑/↓ move · Enter open or choose · Space mark · Backspace up · Esc cancel ")
        }
    }
}

/// One row of the file picker, `width` cells wide: a gutter holding a check
/// mark if the file is `marked`, the name -- a folder's ends in `/`, and a
/// name that would run into the columns on the right is cut short with an
/// ellipsis -- and, for a file, its size and when it was last modified
/// (`modified_label`, against `now`). A `highlighted` row is shown black on
/// cyan across its whole width, like the switcher's, so it carries no
/// colors of its own.
pub(super) fn picker_row(
    entry: &Entry,
    marked: bool,
    highlighted: bool,
    width: usize,
    now: DateTime<Local>,
) -> ListItem<'static> {
    // The size and modified columns, two blanks between them, and a blank
    // at the right edge.
    let columns_width = PICKER_SIZE_WIDTH + 2 + PICKER_TIME_WIDTH + 1;
    let name_width = width.saturating_sub(PICKER_GUTTER + columns_width).max(1);
    let styled = |text: String, style: Style| {
        if highlighted {
            Span::raw(text)
        } else {
            Span::styled(text, style)
        }
    };

    let gutter = if marked { " ✓ " } else { "   " };
    let label = match entry.kind {
        Kind::Folder => format!("{}/", entry.name),
        Kind::File { .. } => entry.name.clone(),
    };
    let name_style = match entry.kind {
        Kind::Folder => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        Kind::File { .. } => Style::default(),
    };
    let mut spans = vec![
        styled(
            gutter.to_string(),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        styled(
            format!("{:<name_width$}", truncate_chars(&label, name_width)),
            name_style,
        ),
    ];
    if let Kind::File { size, modified } = entry.kind {
        spans.push(styled(
            format!(
                "{:>PICKER_SIZE_WIDTH$}  {}",
                human_size(size),
                modified_label(modified, now)
            ),
            Style::default().fg(Color::DarkGray),
        ));
    }
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

/// When a file was last modified, in `PICKER_TIME_WIDTH` cells -- see
/// `day_label`.
pub(super) fn modified_label(modified: SystemTime, now: DateTime<Local>) -> String {
    day_label(DateTime::<Local>::from(modified), now)
}

/// A moment as `PICKER_TIME_WIDTH` cells say it, against `now`: the day and
/// time for the last half year (`12 Oct 14:03`), the day and year before
/// that (`03 Mar  2025`) -- as `ls -l` does, since a time says little about
/// something long ago. What the picker shows for a file's modification and
/// the files overlay for its share (`sent_label`).
pub(super) fn day_label(when: DateTime<Local>, now: DateTime<Local>) -> String {
    let recent = now.signed_duration_since(when) < chrono::Duration::days(180);
    when.format(if recent { "%d %b %H:%M" } else { "%d %b  %Y" })
        .to_string()
}

/// The files overlay opened by `/files` (`AppState::viewing_files`), drawn
/// over everything else like the picker: a cleared, bordered box titled with
/// the channel whose files it lists, holding the typed filter and, under it,
/// every file that matches (`AppState::files_matches`) -- newest first, a
/// row per share (`files_row`) -- with the highlighted row in the switcher's
/// style. Its size is fixed, so the box doesn't jump around as typing
/// narrows the list; it scrolls to keep the highlight in view
/// Splits the RHS column into the peers list and, when `show_hints` is
/// on, a command reference panel below it (toggled via `/hints` -- see
/// `app::AppState::run_hints`). Peers keeps using whatever room is
/// available (`Constraint::Min`), same as when the hints panel is
/// hidden; the hints panel only ever takes exactly the room its fixed
/// content needs (`Constraint::Length`).
pub(super) fn render_sidebar(frame: &mut Frame, area: Rect, app: &AppState) {
    if !app.show_hints {
        render_peers(frame, area, app);
        return;
    }
    let [peers, hints] = sidebar_areas(area);
    render_peers(frame, peers, app);
    render_hints(frame, hints);
}
