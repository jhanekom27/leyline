//! Pure rendering: reads `AppState`, writes widgets to the frame. No
//! mutation, no I/O -- see concept.md's "one owner of UI state" principle.

use std::time::SystemTime;

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::scrollbar;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};
use textwrap::wrap;

use crate::app::{AppState, Channel, TranscriptLine, hex_id};
use crate::files::{StagedAttachment, human_size};
use crate::hyperlink;
use crate::markdown;
use crate::message::ChatMessage;
use crate::picker::{Entry, FilePicker, Kind};
use crate::search::SearchResults;
use crate::thread::ThreadView;

/// Width of the `HH:MM` time column in a chat row.
const TIME_WIDTH: usize = 5;
/// Width of the sender-name column in a chat row; longer names are
/// truncated with an ellipsis so the `│` separator after it always lines
/// up (see `fit_name`).
const NAME_WIDTH: usize = 10;
/// Separates the name column from the message text.
const SEPARATOR: &str = " │ ";

/// Maximum number of input lines shown at once before the box scrolls
/// vertically (see `render_input`) -- keeps a long composed or pasted
/// message from growing the input box to swallow the whole terminal.
const MAX_INPUT_VISIBLE_LINES: usize = 6;

/// Most rows the staged-attachments strip (`render_staged`) shows at once
/// -- keeps a big multi-file stage from pushing the transcript off screen.
/// When more are staged, the oldest collapse into a single "and N more"
/// row (`staged_rows`).
const MAX_STAGED_ROWS: usize = 4;

/// Width of the navigation panel on the left (`render_nav`), borders
/// included.
const NAV_WIDTH: u16 = 24;
/// Terminal cells every channel's kind glyph is padded to, so all labels
/// start in one column whichever kind they follow: the widest glyph (the
/// point-to-point bolt, see `ChannelKind::glyph`) takes two.
const NAV_GLYPH_WIDTH: usize = 2;
/// Where a label starts within a navigation row: a leading space, the
/// glyph column, and a separating space.
const NAV_LABEL_OFFSET: usize = 1 + NAV_GLYPH_WIDTH + 1;
/// Cells held back at a row's right edge for the unread marker (the `*`
/// itself plus a blank before it), so a long label never runs into it.
const NAV_MARKER_WIDTH: usize = 2;

/// Width of the jump-to overlay (`render_switcher`), borders included.
const SWITCHER_WIDTH: u16 = 44;
/// Most matches the jump-to overlay shows at once before it scrolls.
const SWITCHER_MAX_ROWS: usize = 10;

/// Width of the file picker overlay (`render_picker`), borders included.
const PICKER_WIDTH: u16 = 78;
/// Rows of entries the file picker lists at once before it scrolls.
const PICKER_ROWS: usize = 14;
/// Cells at the left of a picker row, where a marked file's check mark goes.
const PICKER_GUTTER: usize = 3;
/// Width of a picker row's size column: room for `1023.9 KB`.
const PICKER_SIZE_WIDTH: usize = 9;
/// Width of a picker row's modified column: `12 Oct 14:03`, `03 Mar  2025`.
const PICKER_TIME_WIDTH: usize = 12;

/// Short usage reminders for every slash command, shown in the sidebar's
/// commands panel (`render_hints`) when `AppState::show_hints` is on.
/// Kept in sync with the Commands table in README.md and `run_help`'s
/// system notice.
const COMMAND_HINTS: &[&str] = &[
    "/join <name|ticket>",
    "/invite",
    "/msg <alias|hex> [text]",
    "/leave [channel]",
    "/rename <name>",
    "/who",
    "/send [path]",
    "/save [hash]",
    "/paste (or Ctrl+V)",
    "/alias <hex> <name>",
    "/nick <name>",
    "/search <term> (/s)",
    "/reply [text]",
    "/thread (Ctrl+T pick)",
    "/hints",
    "/bell",
    "/version",
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
    let [header, body, staged, input] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(staged_strip_height(app.active().staged.len())),
        Constraint::Length(input_box_height(&app.active().input)),
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    render_body(frame, body, app);
    render_staged(frame, staged, app);
    render_input(frame, input, app);
    // Last, so the overlay sits over everything else -- and so its cursor
    // wins over the input box's. At most one is ever open: each owns every
    // key while it is.
    render_switcher(frame, app);
    render_picker(frame, app);
}

/// Height (in terminal rows, including the 2 border rows) the input box
/// needs for `input`'s current line count, capped at
/// `MAX_INPUT_VISIBLE_LINES` -- computed before laying out the rest of
/// the frame (`render`) so the message pane shrinks to make room while
/// composing a multi-line message, and grows back once it's sent.
fn input_box_height(input: &str) -> u16 {
    let lines = input.split('\n').count().max(1);
    lines.min(MAX_INPUT_VISIBLE_LINES) as u16 + 2
}

/// Height (in terminal rows, including the 2 border rows) of the strip of
/// staged attachments above the input box -- zero when nothing is staged,
/// so the strip takes no room until there's something to show. Computed
/// before laying out the rest of the frame (`render`), like
/// `input_box_height`, so the message pane gives up exactly that much.
fn staged_strip_height(staged: usize) -> u16 {
    if staged == 0 {
        0
    } else {
        staged.min(MAX_STAGED_ROWS) as u16 + 2
    }
}

/// The one-row strip above the body: who you are (your full endpoint id,
/// which a peer can paste into `/dm` to reach you directly) and, once a
/// peer has been seen running a newer build, a standing update notice. The
/// channel list lives in the navigation panel (`render_nav`).
fn render_header(frame: &mut Frame, area: Rect, app: &AppState) {
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

fn render_body(frame: &mut Frame, area: Rect, app: &AppState) {
    let [nav, messages, sidebar] = Layout::horizontal([
        Constraint::Length(NAV_WIDTH),
        Constraint::Min(20),
        Constraint::Length(22),
    ])
    .areas(area);

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
/// ends in a `*`. Both section headers are always drawn, even over an
/// empty section, so the panel's shape doesn't jump as conversations come
/// and go.
///
/// When the rows outnumber the panel's height it scrolls to keep the
/// active row in view (`nav_scroll_offset`), with a thumb on its right
/// border (`render_scroll_indicator`).
fn render_nav(frame: &mut Frame, area: Rect, app: &AppState) {
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
fn nav_section_header(title: &str) -> ListItem<'static> {
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
/// `NAV_MARKER_WIDTH` cells free, and -- right-aligned -- a `*` when the
/// channel has unseen activity. A `highlighted` row -- the active channel
/// in the navigation panel, the selected match in the switcher -- is shown
/// black on cyan across the whole row and carries no `*`: it's the channel
/// you're on, or about to be.
fn nav_entry(app: &AppState, index: usize, width: usize, highlighted: bool) -> ListItem<'static> {
    let channel = &app.channels[index];

    let glyph = Span::raw(app.channel_kind(&channel.name).glyph());
    let padding = " ".repeat(NAV_GLYPH_WIDTH.saturating_sub(glyph.width()) + 1);
    let label = truncate_chars(
        &app.channel_label(&channel.name),
        width.saturating_sub(NAV_LABEL_OFFSET + NAV_MARKER_WIDTH),
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
    if channel.has_unread {
        let gap = width.saturating_sub(line.width() + 1);
        line.push_span(Span::raw(" ".repeat(gap)));
        line.push_span(Span::styled(
            "*",
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
fn nav_scroll_offset(total: usize, visible: usize, active_row: usize) -> usize {
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
fn render_switcher(frame: &mut Frame, app: &AppState) {
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

/// Where the switcher box goes on a `screen`-sized frame when `matches`
/// channels are listed: `SWITCHER_WIDTH` wide, tall enough for up to
/// `SWITCHER_MAX_ROWS` matches (one row even with none, for the
/// placeholder), and shrinking as typing narrows the list -- see
/// `overlay_area`.
fn switcher_area(screen: Rect, matches: usize) -> Rect {
    overlay_area(screen, SWITCHER_WIDTH, matches.clamp(1, SWITCHER_MAX_ROWS))
}

/// Where an overlay `width` wide, with room for `list_rows` rows under its
/// borders and its filter row, goes on a `screen`-sized frame: centered, and
/// anchored near the top so the box doesn't jump around as typing changes
/// how many rows it needs. Never larger than the screen. Shared by the
/// switcher and the file picker, which are laid out the same way.
fn overlay_area(screen: Rect, width: u16, list_rows: usize) -> Rect {
    let width = width.min(screen.width);
    let height = (list_rows as u16 + 3).min(screen.height);
    let x = screen.x + (screen.width - width) / 2;
    let y = (screen.y + 2).min(screen.bottom() - height);
    Rect::new(x, y, width, height)
}

/// An overlay's filter row: a prompt and the tail of what's been typed --
/// the end is what's being edited -- with the terminal cursor just after
/// it.
fn render_filter_row(frame: &mut Frame, area: Rect, query: &str) {
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
fn render_picker(frame: &mut Frame, app: &AppState) {
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
fn picker_area(screen: Rect) -> Rect {
    overlay_area(screen, PICKER_WIDTH, PICKER_ROWS)
}

/// The picker's title: the folder being browsed -- its tail, if it won't all
/// fit -- then how many files are marked and whether a folder is still being
/// read. `width` is the box's, borders included.
fn picker_title(picker: &FilePicker, width: usize) -> String {
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
fn picker_footer(picker: &FilePicker, width: usize) -> Line<'static> {
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
fn picker_row(
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

/// When a file was last modified, in `PICKER_TIME_WIDTH` cells: the day and
/// time for the last half year (`12 Oct 14:03`), the day and year before
/// that (`03 Mar  2025`) -- as `ls -l` does, since a time says little about
/// a file from long ago.
fn modified_label(modified: SystemTime, now: DateTime<Local>) -> String {
    let modified = DateTime::<Local>::from(modified);
    let recent = now.signed_duration_since(modified) < chrono::Duration::days(180);
    modified
        .format(if recent { "%d %b %H:%M" } else { "%d %b  %Y" })
        .to_string()
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
    let heading = app.channel_heading(&channel.name);
    let visible_rows = area.height.saturating_sub(2).max(1) as usize; // minus borders
    let inner_width = (area.width as usize).saturating_sub(2).max(1); // minus borders

    // Flattened one-`Line`-per-terminal-row, not one per `TranscriptLine`
    // -- a wrapped message now spans several rows, so the scroll window
    // below has to work in rows to stay accurate. `picked_row` is the
    // header row of whichever message matches `app.picking_reply`, if
    // any -- see the window adjustment below.
    let (mut rows, picked_row) = build_message_rows(app, channel, inner_width);
    let total = rows.len();
    // `channel.scroll` grows unboundedly while `Up` is held (see
    // `AppState::handle_key`) since it has no way to know how many rows
    // of *wrapped* content exist -- that depends on the terminal width,
    // which is only known here. Clamp it so scrolling past the oldest
    // message just holds the view there instead of pushing every row
    // off-screen.
    let max_scroll = total.saturating_sub(visible_rows);
    let scroll = channel.scroll.min(max_scroll);
    let mut end = total.saturating_sub(scroll);
    let mut start = end.saturating_sub(visible_rows);
    // While picking a reply target, the highlighted message always wins
    // over `channel.scroll` -- widen/shift the window (local variables
    // only; `channel.scroll` itself is never touched, since ui.rs stays
    // read-only) just enough to bring it into view, rather than leaving
    // the user to scroll manually to find whatever `Up`/`Down` just
    // selected.
    if let Some(picked) = picked_row {
        if picked < start {
            start = picked;
            end = (start + visible_rows).min(total);
        } else if picked >= end {
            end = (picked + 1).min(total);
            start = end.saturating_sub(visible_rows);
        }
    }
    let items: Vec<ListItem> = rows.drain(start..end).map(ListItem::new).collect();

    let title = if app.picking_reply.is_some() {
        "pick a message to reply to: ↑/↓ move · Enter confirm · Esc cancel".to_string()
    } else if app.picking_thread.is_some() {
        "pick a message to view its thread: ↑/↓ move · Enter confirm · Esc cancel".to_string()
    } else if let Some(thread) = &channel.thread {
        format!(
            "{} · thread from {}: {} ({}) · /thread or Esc to leave",
            heading,
            app.display_name(&thread.origin.sender),
            reply_snippet(&thread.origin),
            thread.messages.len()
        )
    } else {
        match &channel.search {
            Some(search) => {
                let status = if search.pending {
                    ", searching full history..."
                } else {
                    ""
                };
                format!(
                    "{} · search '{}' ({}{status}) · /s to clear",
                    heading,
                    search.term,
                    search.messages.len()
                )
            }
            None => format!("{} · {} peer(s)", heading, channel.peers.len()),
        }
    };
    let block = Block::bordered()
        .title(title)
        .border_type(BorderType::Rounded);
    let inner = block.inner(area);
    let list = List::new(items).block(block);
    frame.render_widget(list, area);
    render_scroll_indicator(frame, area, start, total, visible_rows);
    hyperlink::linkify(frame.buffer_mut(), inner);
}

/// Overlays a scroll-position thumb on a bordered block's right border --
/// the messages pane's (how far back you've scrolled and how much history
/// is above) and the navigation panel's (how much of the channel list is
/// out of view). `start` is the same top-of-viewport row index the caller
/// used to slice the visible window. Only drawn once the content actually
/// overflows one screen (`total > visible_rows`) -- a full-height thumb on
/// a short list would just be noise.
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
/// Delegates to `build_thread_rows` or `build_search_rows` when `channel`
/// has an active `/thread` view or `/search` (see `app::Channel::thread`/
/// `search`), rendering one of those in place of the normal transcript --
/// a channel shows at most one alternate view at a time, so `thread` is
/// checked first but the two never both apply.
fn build_message_rows(
    app: &AppState,
    channel: &Channel,
    inner_width: usize,
) -> (Vec<Line<'static>>, Option<usize>) {
    if let Some(thread) = &channel.thread {
        return build_thread_rows(app, thread, inner_width);
    }
    if let Some(search) = &channel.search {
        return build_search_rows(app, search, inner_width);
    }

    let layout = row_layout(inner_width);
    let mut rows = Vec::new();
    let mut picked_row = None;
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
                let is_picked =
                    app.picking_reply == Some(message.id) || app.picking_thread == Some(message.id);
                if is_picked {
                    picked_row = Some(rows.len());
                }
                push_chat_rows(
                    &mut rows,
                    app,
                    message,
                    is_first_of_group,
                    &layout,
                    None,
                    is_picked,
                );
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
                for chunk in markdown::wrap_chars(&format!("* {text}"), inner_width) {
                    rows.push(Line::from(Span::styled(chunk, style)));
                }
            }
        }
    }
    (rows, picked_row)
}

/// Renders a channel's active `/search` results in place of its normal
/// transcript -- reuses the same per-message formatting
/// (`push_chat_rows`), grouped the same way, but with no `System` lines to
/// interleave since a search snapshot only ever holds chat matches.
/// Highlights the matched substring in each line (see `highlight_spans`).
fn build_search_rows(
    app: &AppState,
    search: &SearchResults,
    inner_width: usize,
) -> (Vec<Line<'static>>, Option<usize>) {
    let layout = row_layout(inner_width);
    let term_lower = search.term.to_lowercase();

    let mut rows = Vec::new();
    let mut picked_row = None;
    let mut last_sender: Option<[u8; 32]> = None;
    for message in &search.messages {
        let is_first_of_group = last_sender != Some(message.sender);
        if is_first_of_group && last_sender.is_some() {
            rows.push(Line::from(""));
        }
        last_sender = Some(message.sender);
        let is_picked =
            app.picking_reply == Some(message.id) || app.picking_thread == Some(message.id);
        if is_picked {
            picked_row = Some(rows.len());
        }
        push_chat_rows(
            &mut rows,
            app,
            message,
            is_first_of_group,
            &layout,
            Some(&term_lower),
            is_picked,
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
    (rows, picked_row)
}

/// Renders a channel's active `/thread` view in place of its normal
/// transcript -- reuses the same per-message formatting
/// (`push_chat_rows`) and sender-grouping as the normal transcript and
/// search results, but never highlights anything (`term_lower: None`),
/// since a thread view isn't about a text match. Always has at least one
/// row, since `thread::build_chain` always includes the origin message.
fn build_thread_rows(
    app: &AppState,
    thread: &ThreadView,
    inner_width: usize,
) -> (Vec<Line<'static>>, Option<usize>) {
    let layout = row_layout(inner_width);

    let mut rows = Vec::new();
    let mut picked_row = None;
    let mut last_sender: Option<[u8; 32]> = None;
    for message in &thread.messages {
        let is_first_of_group = last_sender != Some(message.sender);
        if is_first_of_group && last_sender.is_some() {
            rows.push(Line::from(""));
        }
        last_sender = Some(message.sender);
        let is_picked =
            app.picking_reply == Some(message.id) || app.picking_thread == Some(message.id);
        if is_picked {
            picked_row = Some(rows.len());
        }
        push_chat_rows(
            &mut rows,
            app,
            message,
            is_first_of_group,
            &layout,
            None,
            is_picked,
        );
    }
    (rows, picked_row)
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
/// name, separator, then the message's markdown-rendered body -- see
/// `crate::markdown::render` -- with any active search match additionally
/// highlighted on top via `highlight_spans`) to `rows`. Blanks the
/// time/name on continuation lines and on a run of messages from the same
/// sender (`is_first_of_group`, computed by the caller since it depends
/// on iterating the surrounding transcript/search results in order).
/// Shared by the normal transcript (`build_message_rows`) and search
/// results (`build_search_rows`); `term_lower` is `None` for the former,
/// which never highlights anything.
fn push_chat_rows(
    rows: &mut Vec<Line<'static>>,
    app: &AppState,
    message: &ChatMessage,
    is_first_of_group: bool,
    layout: &RowLayout,
    term_lower: Option<&str>,
    is_picked: bool,
) {
    let color = user_color(&message.sender);
    let name = fit_name(&app.display_name(&message.sender), NAME_WIDTH);
    let time = format_time(message.ts_unix_ms);

    // What was typed -- an ordinary message's whole body, or the caption of
    // a shared file -- goes through `markdown::render`; a captionless file
    // has none. A file's own line follows it, and is synthesized plain
    // text (a filename and a size), never markdown.
    let attachment = attachment_line(app, message);
    let mut bodies: Vec<Vec<Span<'static>>> = Vec::new();
    if !message.text.is_empty() || attachment.is_none() {
        bodies.extend(
            markdown::render(&message.text, layout.wrap_width)
                .into_iter()
                .map(|row| highlight_spans(row, term_lower)),
        );
    }
    if let Some(line) = &attachment {
        let mut wrapped = wrap(line, layout.wrap_width);
        if wrapped.is_empty() {
            wrapped.push(std::borrow::Cow::Borrowed(""));
        }
        bodies.extend(
            wrapped
                .iter()
                .map(|chunk| highlighted_spans(chunk, term_lower)),
        );
    }

    // A reply's quoted preview (`reply_preview_spans`), if any, renders as
    // its own single, unwrapped row ahead of the message's own text -- it
    // takes this group's header (time/name) exactly like the text's own
    // first line would otherwise (via the `index == 0` check below), and
    // the real text always starts on a "continuation" row instead, the
    // same as any second row already does.
    if let Some(reply_to) = message.reply_to {
        bodies.insert(0, reply_preview_spans(app, reply_to));
    }

    for (index, body) in bodies.into_iter().enumerate() {
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
        spans.extend(body);
        if is_picked {
            spans = spans.into_iter().map(reversed_span).collect();
        }
        rows.push(Line::from(spans));
    }
}

/// Builds the display line for a message's file attachment, if any --
/// filename, human-readable size, and (for anyone else's message) the
/// exact `/save` command to fetch it. Your own messages omit the hint
/// since the file is already local. `None` for a plain text message.
/// `push_chat_rows` shows it after the message's own text, which for a
/// shared file is the caption its sender typed, if any.
fn attachment_line(app: &AppState, message: &ChatMessage) -> Option<String> {
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

/// Renders `text` as one or more plain spans, styling matched segments
/// (see `find_highlights`) distinctly. A thin wrapper around
/// `highlight_spans` for the common case of a single unstyled string --
/// used for attachment captions, which are synthesized text rather than
/// an actual message body. `term_lower` is `None` for the normal
/// transcript, which never highlights anything.
fn highlighted_spans(text: &str, term_lower: Option<&str>) -> Vec<Span<'static>> {
    highlight_spans(vec![Span::raw(text.to_string())], term_lower)
}

/// Overlays a search-match highlight onto an already-styled row of spans,
/// so a match inside markdown-rendered text (bold, inline code, a
/// heading, ...) keeps that styling instead of it being replaced.
/// Intersects each span's char range against `find_highlights`'s match
/// segments (computed from the row's concatenated plain text) and, for
/// the matched portions, layers the highlight style over the span's own
/// via `Style::patch` (which fills in the highlight's fg/bg/modifiers,
/// leaving anything it doesn't set untouched). `term_lower` is `None` for
/// the normal transcript, which never highlights anything -- `spans` is
/// returned unchanged in that case, and whenever nothing actually matches.
fn highlight_spans(spans: Vec<Span<'static>>, term_lower: Option<&str>) -> Vec<Span<'static>> {
    let Some(term_lower) = term_lower else {
        return spans;
    };
    let full_text: String = spans.iter().map(|span| span.content.as_ref()).collect();
    let segments = find_highlights(&full_text, term_lower);
    if let [(_, false)] = segments.as_slice() {
        return spans; // no match anywhere in this row
    }

    let highlight = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    // Char-index bounds of each match segment, to intersect against each
    // span's own bounds below.
    let mut bounds = Vec::with_capacity(segments.len());
    let mut offset = 0;
    for (text, is_match) in &segments {
        let len = text.chars().count();
        bounds.push((offset, offset + len, *is_match));
        offset += len;
    }

    let mut out = Vec::new();
    let mut span_start = 0;
    for span in spans {
        let chars: Vec<char> = span.content.chars().collect();
        let span_end = span_start + chars.len();
        for &(seg_start, seg_end, is_match) in &bounds {
            let start = seg_start.max(span_start);
            let end = seg_end.min(span_end);
            if start >= end {
                continue;
            }
            let piece: String = chars[start - span_start..end - span_start].iter().collect();
            let style = if is_match {
                span.style.patch(highlight)
            } else {
                span.style
            };
            out.push(Span::styled(piece, style));
        }
        span_start = span_end;
    }
    out
}

/// Maximum length (in characters) of the quoted snippet shown for a reply
/// -- both above the reply itself (`reply_preview_spans`) and in the input
/// box's title while one is armed (`reply_banner_title`). Longer text is
/// truncated with an ellipsis (`truncate_chars`) so the preview always
/// stays a single, unwrapped row.
const REPLY_SNIPPET_MAX: usize = 40;

/// Builds a reply's quoted-preview row: the parent message's sender and a
/// truncated snippet of its text (see `reply_snippet`), styled dim/italic
/// so it reads as context rather than part of the reply itself. Looked up
/// in the active channel (`Channel::find_message`) -- if the parent isn't
/// found there (evicted from scrollback, not yet backfilled, or simply
/// unknown -- see `ChatMessage::reply_to`'s doc comment), renders an
/// explicit "unavailable" fallback instead of silently dropping the
/// relationship.
fn reply_preview_spans(app: &AppState, reply_to: u64) -> Vec<Span<'static>> {
    let style = Style::default()
        .fg(Color::DarkGray)
        .add_modifier(Modifier::ITALIC);
    let text = match app.active().find_message(reply_to) {
        Some(parent) => format!(
            "↳ {}: {}",
            app.display_name(&parent.sender),
            reply_snippet(parent)
        ),
        None => "↳ replying to a message that's no longer available".to_string(),
    };
    vec![Span::styled(text, style)]
}

/// A short, single-line, quoted excerpt of `message`'s text (or its
/// attachment's filename, for a captionless file share) -- used for a
/// reply's quoted preview (`reply_preview_spans`) and the input box's
/// title while a reply is armed (`reply_banner_title`). Truncated to
/// `REPLY_SNIPPET_MAX` characters with an ellipsis so it can never wrap.
fn reply_snippet(message: &ChatMessage) -> String {
    let text = match &message.attachment {
        Some(attachment) if message.text.is_empty() => attachment.filename.as_str(),
        _ => message.text.as_str(),
    };
    // Flattened to a single line first -- both callers (the reply preview
    // row and the input box's reply banner) need this to stay one
    // unwrapped row, which a multi-line parent's embedded newline would
    // otherwise break.
    let flattened = text.replace('\n', " ");
    format!("\"{}\"", truncate_chars(&flattened, REPLY_SNIPPET_MAX))
}

/// Truncates `text` to at most `max_chars` characters, appending `…` if it
/// was longer. Unlike `fit_name`, never pads a shorter string -- this is
/// for prose (a reply's quoted snippet), not a fixed-width column.
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{truncated}…")
}

/// Truncates `text` to at most `max_chars` characters by dropping its
/// start, prefixing `…` if it was longer -- for a path, whose end is the
/// part that tells you where you are.
fn truncate_start(text: &str, max_chars: usize) -> String {
    let len = text.chars().count();
    if len <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let tail: String = text.chars().skip(len - (max_chars - 1)).collect();
    format!("…{tail}")
}

/// Adds a reversed-video modifier to `span`, preserving its existing
/// colors -- used to highlight the currently-picked message's row(s)
/// while choosing a reply target (`push_chat_rows`'s `is_picked`). Patched
/// per-span (rather than once on the whole `Line`) so it always takes
/// effect regardless of each span's own explicit styling.
fn reversed_span(span: Span<'static>) -> Span<'static> {
    Span {
        style: span.style.add_modifier(Modifier::REVERSED),
        ..span
    }
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
    // One row per person, not per device -- see `AppState::unique_peer_ids`.
    let peers = app.unique_peer_ids(&channel.peers);
    let mut items: Vec<ListItem> = peers.iter().map(|id| peer_item(app, id)).collect();
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
fn render_staged(frame: &mut Frame, area: Rect, app: &AppState) {
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
fn staged_rows(staged: &[StagedAttachment], width: usize, marker: Color) -> Vec<Line<'static>> {
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
fn staged_row(item: &StagedAttachment, width: usize, marker: Color) -> Line<'static> {
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
fn cursor_line_and_col(input: &str, cursor: usize) -> (usize, usize) {
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
fn reply_banner_title(app: &AppState, reply_to: u64) -> String {
    match app.active().find_message(reply_to) {
        Some(parent) => format!(
            " replying to {}: {} · Esc to cancel ",
            app.display_name(&parent.sender),
            reply_snippet(parent)
        ),
        None => " replying to a message that's no longer available · Esc to cancel ".to_string(),
    }
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
            reply_to: None,
        }
    }

    fn text_message(id: u64, sender: [u8; 32], text: &str) -> ChatMessage {
        ChatMessage {
            v: 3,
            id,
            sender,
            ts_unix_ms: 0,
            text: text.to_string(),
            attachment: None,
            reply_to: None,
        }
    }

    #[test]
    fn attachment_line_includes_a_save_hint_for_someone_elses_file() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = file_message([2; 32], "report.pdf", 2_150_000);

        let line = attachment_line(&app, &message).unwrap();

        assert!(line.contains("report.pdf"), "got: {line}");
        assert!(line.contains("2.1 MB"), "got: {line}");
        assert!(line.contains("/save"), "got: {line}");
    }

    #[test]
    fn attachment_line_omits_the_save_hint_for_your_own_file() {
        let self_id = [1; 32];
        let app = AppState::new(self_id, vec!["general".to_string()], "general");
        let message = file_message(self_id, "report.pdf", 10);

        let line = attachment_line(&app, &message).unwrap();

        assert!(line.contains("report.pdf"));
        assert!(!line.contains("/save"), "got: {line}");
    }

    #[test]
    fn attachment_line_is_none_for_a_plain_text_message() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = ChatMessage {
            v: 2,
            id: 1,
            sender: [2; 32],
            ts_unix_ms: 0,
            text: "hello".to_string(),
            attachment: None,
            reply_to: None,
        };

        assert_eq!(attachment_line(&app, &message), None);
    }

    #[test]
    fn a_shared_files_caption_is_shown_above_its_attachment_line() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let mut message = file_message([2; 32], "report.pdf", 2_150_000);
        message.text = "the numbers from Q3".to_string();
        let mut rows = Vec::new();

        push_chat_rows(
            &mut rows,
            &app,
            &message,
            true,
            &row_layout(80),
            None,
            false,
        );

        let text: Vec<String> = rows.iter().map(row_text).collect();
        let caption = text
            .iter()
            .position(|row| row.contains("the numbers from Q3"));
        let file = text
            .iter()
            .position(|row| row.contains("shared report.pdf"));
        assert!(
            caption.is_some() && file.is_some() && caption < file,
            "expected the caption row, then the file row; got: {text:?}"
        );
    }

    #[test]
    fn a_captionless_file_shows_only_its_attachment_line() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = file_message([2; 32], "report.pdf", 2_150_000);
        let mut rows = Vec::new();

        push_chat_rows(
            &mut rows,
            &app,
            &message,
            true,
            &row_layout(80),
            None,
            false,
        );

        assert_eq!(rows.len(), 1);
        assert!(row_text(&rows[0]).contains("shared report.pdf"));
    }

    fn staged_item(filename: &str, size: u64) -> StagedAttachment {
        StagedAttachment {
            filename: filename.to_string(),
            size,
            source: crate::files::StagedSource::Bytes(Vec::new()),
        }
    }

    #[test]
    fn staged_strip_takes_no_room_when_nothing_is_staged_and_is_capped_otherwise() {
        assert_eq!(staged_strip_height(0), 0);
        assert_eq!(staged_strip_height(1), 3, "one row plus the two borders");
        let full = MAX_STAGED_ROWS as u16 + 2;
        assert_eq!(staged_strip_height(MAX_STAGED_ROWS), full);
        assert_eq!(staged_strip_height(MAX_STAGED_ROWS + 10), full);
    }

    #[test]
    fn staged_rows_show_each_attachments_name_and_size() {
        let staged = [
            staged_item("report.pdf", 2_150_000),
            staged_item("map.png", 480),
        ];

        let rows = staged_rows(&staged, 60, Color::Cyan);

        assert_eq!(rows.len(), 2);
        let first = row_text(&rows[0]);
        assert!(
            first.contains("report.pdf") && first.contains("2.1 MB"),
            "got: {first}"
        );
        let second = row_text(&rows[1]);
        assert!(
            second.contains("map.png") && second.contains("480 B"),
            "got: {second}"
        );
    }

    #[test]
    fn staged_rows_collapse_the_oldest_so_the_newest_stays_visible() {
        let staged: Vec<StagedAttachment> = (1..=6)
            .map(|n| staged_item(&format!("file{n}.txt"), 1))
            .collect();

        let rows = staged_rows(&staged, 60, Color::Cyan);

        assert_eq!(rows.len(), MAX_STAGED_ROWS);
        assert!(row_text(&rows[0]).contains("and 3 more"), "got: {rows:?}");
        assert!(row_text(&rows[MAX_STAGED_ROWS - 1]).contains("file6.txt"));
        assert!(
            !rows.iter().any(|row| row_text(row).contains("file1.txt")),
            "the oldest is folded into the overflow row"
        );
    }

    #[test]
    fn staged_rows_truncate_a_long_name_but_keep_the_size() {
        let long = "a-very-long-file-name-that-will-not-fit-in-the-strip.png";

        let rows = staged_rows(&[staged_item(long, 480)], 30, Color::Cyan);

        let text = row_text(&rows[0]);
        assert!(text.chars().count() <= 30, "got: {text}");
        assert!(text.contains('…'), "got: {text}");
        assert!(text.ends_with("480 B"), "got: {text}");
    }

    #[test]
    fn render_draws_the_staged_strip_above_the_input_box() {
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");
        app.stage_attachment("general", staged_item("report.pdf", 2_150_000));

        let rows = render_to_rows(&app);

        let strip = rows
            .iter()
            .position(|row| row.contains("report.pdf"))
            .expect("the staged file is on screen");
        let input = rows
            .iter()
            .position(|row| row.contains("you ›"))
            .expect("the input box is on screen");
        assert!(strip < input, "strip at row {strip}, input at row {input}");
        assert!(rows[strip].contains("2.1 MB"), "got: {}", rows[strip]);
    }

    #[test]
    fn render_draws_no_strip_when_nothing_is_staged() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");

        let rows = render_to_rows(&app);

        assert!(!rows.iter().any(|row| row.contains("staged")));
    }

    #[test]
    fn truncate_chars_leaves_short_text_untouched() {
        assert_eq!(truncate_chars("hi", 10), "hi");
    }

    #[test]
    fn truncate_chars_shortens_long_text_with_an_ellipsis() {
        let truncated = truncate_chars("hello world", 5);
        assert_eq!(truncated.chars().count(), 5);
        assert!(truncated.ends_with('…'), "got: {truncated}");
        assert!(truncated.starts_with("hell"), "got: {truncated}");
    }

    #[test]
    fn reply_snippet_quotes_the_parents_text() {
        let parent = text_message(1, [1; 32], "see you there");
        assert_eq!(reply_snippet(&parent), "\"see you there\"");
    }

    #[test]
    fn reply_snippet_falls_back_to_a_captionless_files_name() {
        let parent = file_message([1; 32], "map.png", 10);
        assert_eq!(reply_snippet(&parent), "\"map.png\"");
    }

    #[test]
    fn reply_preview_spans_quotes_a_resolvable_parent() {
        let mut app = AppState::new([9; 32], vec!["general".to_string()], "general");
        let parent = text_message(1, [1; 32], "are you around later");
        app.load_history("general", vec![parent]);

        let spans = reply_preview_spans(&app, 1);

        assert_eq!(spans.len(), 1);
        let text = spans[0].content.to_string();
        assert!(text.contains("are you around later"), "got: {text}");
    }

    #[test]
    fn reply_preview_spans_falls_back_when_the_parent_is_unavailable() {
        let app = AppState::new([9; 32], vec!["general".to_string()], "general");

        let spans = reply_preview_spans(&app, 999);

        assert_eq!(spans.len(), 1);
        let text = spans[0].content.to_string();
        assert!(text.contains("no longer available"), "got: {text}");
    }

    #[test]
    fn reversed_span_preserves_content_and_adds_the_modifier() {
        let span = Span::styled("hi", Style::default().fg(Color::Red));
        let reversed = reversed_span(span);
        assert_eq!(reversed.content, "hi");
        assert!(reversed.style.add_modifier.contains(Modifier::REVERSED));
        assert_eq!(reversed.style.fg, Some(Color::Red));
    }

    /// Concatenates a rendered row's spans back into plain text, for
    /// assertions that don't care how the row was split into spans.
    fn row_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn push_chat_rows_renders_a_multiline_message_as_multiple_rows() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = text_message(1, [1; 32], "line one\nline two");
        let layout = row_layout(80);
        let mut rows = Vec::new();

        push_chat_rows(&mut rows, &app, &message, true, &layout, None, false);

        assert_eq!(rows.len(), 2);
        assert!(
            row_text(&rows[0]).ends_with("line one"),
            "got: {}",
            row_text(&rows[0])
        );
        assert!(
            row_text(&rows[1]).ends_with("line two"),
            "got: {}",
            row_text(&rows[1])
        );
    }

    #[test]
    fn push_chat_rows_renders_a_heading_with_its_style() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = text_message(1, [1; 32], "# Title");
        let layout = row_layout(80);
        let mut rows = Vec::new();

        push_chat_rows(&mut rows, &app, &message, true, &layout, None, false);

        assert_eq!(rows.len(), 1);
        // The body starts right after the fixed rail/time/name/separator
        // prefix (see `push_chat_rows`'s column layout).
        let body = rows[0].spans.last().expect("a body span");
        assert_eq!(body.content.as_ref(), "Title");
        assert_eq!(body.style.fg, Some(Color::Cyan));
        assert!(body.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn push_chat_rows_renders_inline_bold_and_code_styling() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = text_message(1, [1; 32], "plain **bold** and `code`");
        let layout = row_layout(80);
        let mut rows = Vec::new();

        push_chat_rows(&mut rows, &app, &message, true, &layout, None, false);

        assert_eq!(rows.len(), 1);
        let body = &rows[0].spans[6..]; // past the fixed rail/time/name/separator prefix
        let bold_span = body
            .iter()
            .find(|span| span.content.as_ref() == "bold")
            .expect("a bold span");
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
        let code_span = body
            .iter()
            .find(|span| span.content.as_ref() == "code")
            .expect("a code span");
        assert_eq!(code_span.style.fg, Some(Color::Magenta));
    }

    #[test]
    fn push_chat_rows_renders_a_list_item_and_a_blockquote() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = text_message(1, [1; 32], "- item one\n> quoted");
        let layout = row_layout(80);
        let mut rows = Vec::new();

        push_chat_rows(&mut rows, &app, &message, true, &layout, None, false);

        assert_eq!(rows.len(), 2);
        assert!(
            row_text(&rows[0]).ends_with("\u{2022} item one"),
            "got: {}",
            row_text(&rows[0])
        );
        assert!(
            row_text(&rows[1]).ends_with("\u{2502} quoted"),
            "got: {}",
            row_text(&rows[1])
        );
    }

    #[test]
    fn push_chat_rows_never_markdown_parses_an_attachment_caption() {
        let self_id = [1; 32];
        let app = AppState::new(self_id, vec!["general".to_string()], "general");
        let message = file_message(self_id, "notes_**important**.txt", 10);
        let layout = row_layout(80);
        let mut rows = Vec::new();

        push_chat_rows(&mut rows, &app, &message, true, &layout, None, false);

        let text = row_text(&rows[0]);
        assert!(
            text.contains("**important**"),
            "caption must stay literal, not be markdown-rendered: {text}"
        );
    }

    #[test]
    fn highlight_spans_returns_plain_spans_unchanged_when_search_is_inactive() {
        let spans = vec![Span::styled(
            "bold".to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        assert_eq!(highlight_spans(spans.clone(), None), spans);
    }

    #[test]
    fn highlight_spans_preserves_markdown_styling_on_a_match() {
        let spans = vec![Span::styled(
            "bold".to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        )];

        let highlighted = highlight_spans(spans, Some("old"));

        let matched = highlighted
            .iter()
            .find(|span| span.content.as_ref() == "old")
            .expect("the matched piece");
        assert!(
            matched.style.add_modifier.contains(Modifier::BOLD),
            "must keep its markdown styling"
        );
        assert_eq!(
            matched.style.bg,
            Some(Color::Yellow),
            "must also carry the search highlight"
        );
        let unmatched = highlighted
            .iter()
            .find(|span| span.content.as_ref() == "b")
            .expect("the unmatched piece");
        assert_eq!(unmatched.style.bg, None, "unmatched text isn't highlighted");
    }

    #[test]
    fn highlight_spans_finds_a_match_spanning_two_spans() {
        // "bo" + "ld" concatenates to "bold", so a search for "old" must
        // still be found even though it straddles a span boundary.
        let spans = vec![
            Span::styled(
                "bo".to_string(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("ld".to_string()),
        ];

        let highlighted = highlight_spans(spans, Some("old"));

        let full_text: String = highlighted.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(full_text, "bold");
        assert!(
            highlighted
                .iter()
                .any(|span| span.style.bg == Some(Color::Yellow)),
            "expected at least one highlighted piece"
        );
    }

    #[test]
    fn build_thread_rows_renders_each_message_in_the_chain() {
        let app = AppState::new([9; 32], vec!["general".to_string()], "general");
        let origin = text_message(1, [1; 32], "root");
        let reply = text_message(2, [1; 32], "reply");
        let thread = ThreadView {
            origin: origin.clone(),
            messages: vec![origin, reply],
        };

        let (rows, _) = build_thread_rows(&app, &thread, 80);

        assert_eq!(rows.len(), 2);
        assert!(
            row_text(&rows[0]).ends_with("root"),
            "got: {}",
            row_text(&rows[0])
        );
        assert!(
            row_text(&rows[1]).ends_with("reply"),
            "got: {}",
            row_text(&rows[1])
        );
    }

    #[test]
    fn build_thread_rows_highlights_the_row_matching_picking_thread() {
        let mut app = AppState::new([9; 32], vec!["general".to_string()], "general");
        app.picking_thread = Some(1);
        let origin = text_message(1, [1; 32], "root");
        let thread = ThreadView {
            origin: origin.clone(),
            messages: vec![origin],
        };

        let (rows, picked_row) = build_thread_rows(&app, &thread, 80);

        assert_eq!(picked_row, Some(0));
        assert!(
            rows[0]
                .spans
                .iter()
                .any(|span| span.style.add_modifier.contains(Modifier::REVERSED)),
            "the picked row must be rendered reversed"
        );
    }

    use std::collections::HashMap;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::buffer::Buffer;

    /// Renders `app` into an off-screen terminal of `width` x `height`
    /// cells and returns what was drawn, so a test can assert on what a
    /// user would actually see rather than on the state that drives it.
    fn render_to_buffer(app: &AppState, width: u16, height: u16) -> Buffer {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
                .expect("a test backend never fails");
        terminal
            .draw(|frame| render(frame, app))
            .expect("drawing to a test backend never fails");
        terminal.backend().buffer().clone()
    }

    /// The symbols of `buffer`'s row `y` from column `x` for `width`
    /// cells, one per cell -- so a test can check *which column* something
    /// landed in.
    fn cells(buffer: &Buffer, y: u16, x: u16, width: u16) -> String {
        (x..x + width).map(|x| buffer[(x, y)].symbol()).collect()
    }

    /// `width` columns of `buffer`'s row `y`, from column `x`, as a user
    /// reads them: the blank cell a double-width glyph (the point-to-point
    /// bolt) leaves behind it in the buffer isn't a column of its own on
    /// screen, so it's left out.
    fn visible_row(buffer: &Buffer, y: u16, x: u16, width: u16) -> String {
        let mut text = String::new();
        let mut hidden = 0;
        for x in x..x + width {
            if hidden > 0 {
                hidden -= 1;
                continue;
            }
            let symbol = buffer[(x, y)].symbol();
            hidden = Span::raw(symbol).width().saturating_sub(1);
            text.push_str(symbol);
        }
        text
    }

    /// The text of every screen row of `buffer`.
    fn screen_rows(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| visible_row(buffer, y, 0, buffer.area.width))
            .collect()
    }

    /// The text of every screen row of a 100x20 render of `app`.
    fn render_to_rows(app: &AppState) -> Vec<String> {
        screen_rows(&render_to_buffer(app, 100, 20))
    }

    /// Just the navigation panel's part of each row of `buffer` -- the
    /// message pane's title right beside it repeats channel names.
    fn nav_rows(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| visible_row(buffer, y, 0, NAV_WIDTH))
            .collect()
    }

    /// Where a navigation row's glyph sits: past the border and the
    /// leading space.
    const NAV_GLYPH_X: u16 = 2;
    /// Where a navigation row's label starts, whichever glyph precedes it.
    const NAV_LABEL_X: u16 = NAV_GLYPH_X + NAV_GLYPH_WIDTH as u16 + 1;
    /// Where a navigation row's unread marker sits: the last cell before
    /// the right border.
    const NAV_MARKER_X: u16 = NAV_WIDTH - 2;

    /// The screen row of the entry with `glyph` in the glyph column and
    /// `label` in the label column of the bordered box whose left edge is
    /// at column `left` -- the navigation panel at 0, the switcher wherever
    /// it's centered; both lay their entries out identically. Panics, with
    /// the whole screen, if there's none.
    fn entry_row(buffer: &Buffer, left: u16, glyph: &str, label: &str) -> u16 {
        let label_width = label.chars().count() as u16;
        (0..buffer.area.height)
            .find(|&y| {
                cells(buffer, y, left + NAV_GLYPH_X, 1) == glyph
                    && cells(buffer, y, left + NAV_LABEL_X, label_width) == label
            })
            .unwrap_or_else(|| {
                panic!(
                    "no entry {glyph}{label} at column {left}:\n{}",
                    screen_rows(buffer).join("\n")
                )
            })
    }

    /// `entry_row` for the navigation panel.
    fn nav_row(buffer: &Buffer, glyph: &str, label: &str) -> u16 {
        entry_row(buffer, 0, glyph, label)
    }

    /// An app with `general` active, then -- joined in this order, so the
    /// two kinds are interleaved -- a `/msg` DM, `project-x`, and a `/dm`
    /// point-to-point chat. The DM and the chat are with one peer and both
    /// stored under hex-prefix names, as they'd have been named before the
    /// peer was.
    fn app_with_every_kind() -> AppState {
        let peer = [7; 32];
        let mut app = AppState::new(
            [9; 32],
            vec![
                "general".to_string(),
                "dm-07070707".to_string(),
                "project-x".to_string(),
            ],
            "general",
        );
        app.record_dm_channel(peer, "dm-07070707".to_string());
        app.background_direct_channel("p2p-07070707".to_string(), peer);
        app
    }

    /// `count` group channels named `channel-00`, `channel-01`, ... with
    /// the one at `active` selected -- more than a short terminal's panel
    /// has room for.
    fn app_with_many_channels(count: usize, active: usize) -> AppState {
        let names: Vec<String> = (0..count).map(|i| format!("channel-{i:02}")).collect();
        let active_name = names[active].clone();
        AppState::new([9; 32], names, &active_name)
    }

    #[test]
    fn the_top_row_is_the_identity_line_not_a_tab_bar() {
        let app = app_with_every_kind();

        let rows = render_to_rows(&app);

        let identity = format!("you: {}", hex_id(&[9; 32]));
        assert!(rows[0].contains(&identity), "got: {}", rows[0]);
        assert!(
            !rows[0].contains("general"),
            "channel names belong in the panel, got: {}",
            rows[0]
        );
        assert!(
            rows[1].starts_with("╭leyline"),
            "the panel starts right under the identity line, got: {}",
            rows[1]
        );
    }

    #[test]
    fn the_nav_panel_lists_channels_then_people_under_their_own_headers() {
        let mut app = app_with_every_kind();
        app.load_contacts(HashMap::from([([7; 32], "Alice".to_string())]));

        let rows = nav_rows(&render_to_buffer(&app, 100, 20));

        let row_with = |needle: &str| {
            rows.iter()
                .position(|row| row.contains(needle))
                .unwrap_or_else(|| panic!("no row contains {needle:?}: {rows:#?}"))
        };
        // Group channels first, then the conversations, each in join order
        // -- though the DM was joined before `project-x`.
        let order = [
            row_with("CHANNELS"),
            row_with("general"),
            row_with("project-x"),
            row_with("PEOPLE"),
            row_with("@"),
            row_with("⚡"),
        ];
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "got: {rows:#?}"
        );
    }

    #[test]
    fn both_sections_are_drawn_even_when_there_are_no_conversations() {
        let app = AppState::new([9; 32], vec!["general".to_string()], "general");

        let panel = nav_rows(&render_to_buffer(&app, 100, 20)).join("\n");

        assert!(
            panel.contains("CHANNELS") && panel.contains("PEOPLE"),
            "got:\n{panel}"
        );
    }

    #[test]
    fn each_entry_leads_with_its_kinds_glyph_and_labels_line_up() {
        let mut app = app_with_every_kind();
        app.load_contacts(HashMap::from([([7; 32], "Alice".to_string())]));

        let buffer = render_to_buffer(&app, 100, 20);

        // `nav_row` wants the glyph in its column and the label in its own,
        // which is the same column for the double-width bolt as for the
        // single-width glyphs.
        nav_row(&buffer, "#", "general");
        nav_row(&buffer, "#", "project-x");
        nav_row(&buffer, "@", "Alice");
        nav_row(&buffer, "⚡", "Alice");
    }

    #[test]
    fn the_nav_panel_shows_a_peers_current_alias_not_the_stored_channel_name() {
        let mut app = app_with_every_kind();
        app.load_contacts(HashMap::from([([7; 32], "Alice".to_string())]));

        let panel = nav_rows(&render_to_buffer(&app, 100, 20)).join("\n");

        assert!(!panel.contains("07070707"), "stored name leaked:\n{panel}");
    }

    #[test]
    fn the_nav_panel_shows_the_hex_prefix_until_a_peer_is_named() {
        let app = app_with_every_kind();

        let buffer = render_to_buffer(&app, 100, 20);

        nav_row(&buffer, "@", "07070707");
        nav_row(&buffer, "⚡", "07070707");
    }

    #[test]
    fn the_active_channel_is_highlighted_across_its_whole_row() {
        let mut app = app_with_every_kind();
        app.active = 2; // project-x

        let buffer = render_to_buffer(&app, 100, 20);

        let active = nav_row(&buffer, "#", "project-x");
        let other = nav_row(&buffer, "#", "general");
        for x in 1..NAV_WIDTH - 1 {
            assert_eq!(
                buffer[(x, active)].bg,
                Color::Cyan,
                "column {x} of the active row"
            );
            assert_ne!(
                buffer[(x, other)].bg,
                Color::Cyan,
                "column {x} of another row"
            );
        }
        assert_eq!(buffer[(NAV_LABEL_X, active)].fg, Color::Black);
        assert!(
            buffer[(NAV_LABEL_X, active)]
                .modifier
                .contains(Modifier::BOLD)
        );
    }

    #[test]
    fn a_highlighted_conversation_row_is_highlighted_to_its_edges_too() {
        let mut app = app_with_every_kind();
        app.active = 3; // the /dm chat, behind the double-width bolt

        let buffer = render_to_buffer(&app, 100, 20);

        // The blank cell the bolt leaves behind it carries no style of its
        // own -- the terminal paints the bolt's background across both
        // columns -- so check the cells either side of it.
        let row = nav_row(&buffer, "⚡", "07070707");
        assert_eq!(buffer[(1, row)].bg, Color::Cyan);
        assert_eq!(buffer[(NAV_LABEL_X, row)].bg, Color::Cyan);
        assert_eq!(buffer[(NAV_MARKER_X, row)].bg, Color::Cyan);
    }

    #[test]
    fn an_unread_channel_ends_in_a_marker_but_the_active_one_never_does() {
        let mut app = app_with_every_kind();
        app.channels[2].has_unread = true; // project-x, inactive
        app.channels[0].has_unread = true; // general, active: the flag is moot

        let buffer = render_to_buffer(&app, 100, 20);

        let unread = nav_row(&buffer, "#", "project-x");
        let active = nav_row(&buffer, "#", "general");
        let read = nav_row(&buffer, "@", "07070707");
        assert_eq!(buffer[(NAV_MARKER_X, unread)].symbol(), "*");
        assert_eq!(buffer[(NAV_MARKER_X, active)].symbol(), " ");
        assert_eq!(buffer[(NAV_MARKER_X, read)].symbol(), " ");
    }

    #[test]
    fn a_long_label_is_cut_short_of_the_unread_marker() {
        let long = "a-channel-name-far-too-long-for-the-panel";
        let mut app = AppState::new(
            [9; 32],
            vec!["general".to_string(), long.to_string()],
            "general",
        );
        app.channels[1].has_unread = true;

        let buffer = render_to_buffer(&app, 100, 20);

        // 15 characters and an ellipsis, a blank, then the marker.
        let row = nav_row(&buffer, "#", "a-channel-name-…");
        assert_eq!(buffer[(NAV_MARKER_X - 1, row)].symbol(), " ");
        assert_eq!(buffer[(NAV_MARKER_X, row)].symbol(), "*");
    }

    #[test]
    fn the_nav_panel_scrolls_down_to_keep_the_last_channel_in_view() {
        // A 12-row terminal leaves the panel 6 rows of room (a header row,
        // 3 input-box rows and 2 borders go to everything else), against
        // 14 rows of content: 12 channels and 2 section headers.
        let app = app_with_many_channels(12, 11);

        let panel = nav_rows(&render_to_buffer(&app, 100, 12)).join("\n");

        assert!(panel.contains("channel-11"), "got:\n{panel}");
        assert!(!panel.contains("channel-00"), "got:\n{panel}");
    }

    #[test]
    fn the_nav_panel_scrolls_back_up_to_keep_the_first_channel_in_view() {
        let app = app_with_many_channels(12, 0);

        let panel = nav_rows(&render_to_buffer(&app, 100, 12)).join("\n");

        assert!(panel.contains("channel-00"), "got:\n{panel}");
        assert!(!panel.contains("channel-11"), "got:\n{panel}");
    }

    #[test]
    fn the_nav_panel_keeps_a_middle_channel_in_view_with_neighbours_either_side() {
        let app = app_with_many_channels(12, 6);

        let panel = nav_rows(&render_to_buffer(&app, 100, 12)).join("\n");

        for channel in ["channel-05", "channel-06", "channel-07"] {
            assert!(panel.contains(channel), "{channel} missing:\n{panel}");
        }
        assert!(!panel.contains("channel-00"), "got:\n{panel}");
        assert!(!panel.contains("channel-11"), "got:\n{panel}");
    }

    #[test]
    fn the_nav_panel_draws_a_scroll_thumb_only_when_it_overflows() {
        let has_thumb = |app: &AppState, height: u16| {
            let buffer = render_to_buffer(app, 100, height);
            (0..height).any(|y| buffer[(NAV_WIDTH - 1, y)].symbol() == "▐")
        };

        assert!(has_thumb(&app_with_many_channels(12, 0), 12));
        assert!(!has_thumb(&app_with_many_channels(2, 0), 20));
    }

    #[test]
    fn nav_scroll_offset_is_zero_while_everything_fits() {
        assert_eq!(nav_scroll_offset(5, 10, 4), 0);
        assert_eq!(nav_scroll_offset(10, 10, 9), 0);
    }

    #[test]
    fn nav_scroll_offset_centers_the_active_row_where_there_is_room() {
        // Rows 8..13, with the active row 10 in the middle.
        assert_eq!(nav_scroll_offset(20, 5, 10), 8);
    }

    #[test]
    fn nav_scroll_offset_holds_the_window_against_either_end() {
        assert_eq!(nav_scroll_offset(20, 5, 1), 0);
        assert_eq!(nav_scroll_offset(20, 5, 19), 15);
    }

    #[test]
    fn nav_scroll_offset_always_keeps_the_active_row_inside_the_rows() {
        for total in 1..=30 {
            for visible in 1..=10 {
                for active_row in 0..total {
                    let start = nav_scroll_offset(total, visible, active_row);
                    let context = format!(
                        "total {total}, visible {visible}, active {active_row}: window starts at {start}"
                    );
                    assert!(
                        start <= active_row && active_row < start + visible,
                        "active row out of view: {context}"
                    );
                    assert!(
                        start + visible <= total.max(visible),
                        "window runs past the last row: {context}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_pane_title_names_a_peer_by_its_current_alias_behind_the_kinds_glyph() {
        let mut app = app_with_every_kind();
        app.load_contacts(HashMap::from([([7; 32], "Alice".to_string())]));

        app.active = 1; // the /msg DM
        let dm = render_to_rows(&app);
        app.active = 3; // the /dm chat
        let direct = render_to_rows(&app);

        assert!(
            dm.iter().any(|row| row.contains("@Alice · 0 peer(s)")),
            "got: {dm:#?}"
        );
        assert!(
            direct.iter().any(|row| row.contains("⚡Alice · 0 peer(s)")),
            "got: {direct:#?}"
        );
    }

    #[test]
    fn the_pane_title_shows_a_group_channel_by_its_stored_name() {
        let mut app = app_with_every_kind();
        app.load_contacts(HashMap::from([([7; 32], "Alice".to_string())]));

        let rows = render_to_rows(&app); // `general` is active

        assert!(
            rows.iter().any(|row| row.contains("#general · 0 peer(s)")),
            "got: {rows:#?}"
        );
    }

    /// The screen every switcher render test draws on.
    fn screen() -> Rect {
        Rect::new(0, 0, 100, 20)
    }

    fn open_switcher(app: &mut AppState) {
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    }

    fn type_text(app: &mut AppState, text: &str) {
        for c in text.chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn press(app: &mut AppState, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// The text of the box at `area` on `buffer`, one string per row.
    fn box_rows(buffer: &Buffer, area: Rect) -> Vec<String> {
        (area.y..area.bottom())
            .map(|y| visible_row(buffer, y, area.x, area.width))
            .collect()
    }

    /// Whether row `y` of the box at `area` is highlighted right across
    /// its inside.
    fn row_is_highlighted(buffer: &Buffer, area: Rect, y: u16) -> bool {
        (area.x + 1..area.right() - 1).all(|x| buffer[(x, y)].bg == Color::Cyan)
    }

    #[test]
    fn the_switcher_is_not_drawn_until_it_is_opened() {
        let mut app = app_with_every_kind();
        let closed = render_to_rows(&app);
        assert!(!closed.iter().any(|row| row.contains("jump to")));

        open_switcher(&mut app);

        let open = render_to_rows(&app);
        assert!(open.iter().any(|row| row.contains("jump to")), "{open:#?}");
    }

    #[test]
    fn the_switcher_lists_every_channel_in_panel_order_with_its_glyph() {
        let mut app = app_with_every_kind();
        app.load_contacts(HashMap::from([([7; 32], "Alice".to_string())]));
        open_switcher(&mut app);

        let buffer = render_to_buffer(&app, 100, 20);

        let area = switcher_area(screen(), 4);
        let rows = [
            entry_row(&buffer, area.x, "#", "general"),
            entry_row(&buffer, area.x, "#", "project-x"),
            entry_row(&buffer, area.x, "@", "Alice"),
            entry_row(&buffer, area.x, "⚡", "Alice"),
        ];
        assert!(
            rows.windows(2).all(|pair| pair[0] < pair[1]),
            "got: {rows:?}"
        );
    }

    #[test]
    fn an_unread_channel_keeps_its_marker_in_the_switcher() {
        let mut app = app_with_every_kind();
        app.channels[2].has_unread = true; // project-x
        open_switcher(&mut app);

        let buffer = render_to_buffer(&app, 100, 20);

        let area = switcher_area(screen(), 4);
        let unread = entry_row(&buffer, area.x, "#", "project-x");
        let read = entry_row(&buffer, area.x, "@", "07070707");
        assert_eq!(buffer[(area.right() - 2, unread)].symbol(), "*");
        assert_eq!(buffer[(area.right() - 2, read)].symbol(), " ");
    }

    #[test]
    fn typing_narrows_the_switcher_and_shrinks_its_box() {
        let mut app = app_with_every_kind();
        app.load_contacts(HashMap::from([([7; 32], "Alice".to_string())]));
        open_switcher(&mut app);
        type_text(&mut app, "ALI");

        let buffer = render_to_buffer(&app, 100, 20);

        // Both conversations with Alice match, ignoring case, and nothing
        // else does: two rows, plus the query row and the borders.
        let area = switcher_area(screen(), 2);
        assert_eq!(area.height, 5);
        entry_row(&buffer, area.x, "@", "Alice");
        entry_row(&buffer, area.x, "⚡", "Alice");
        let text = box_rows(&buffer, area).join("\n");
        assert!(
            !text.contains("general") && !text.contains("project-x"),
            "got:\n{text}"
        );
    }

    #[test]
    fn the_highlight_follows_down_and_is_styled_like_the_active_channel() {
        let mut app = app_with_every_kind();
        open_switcher(&mut app);
        let area = switcher_area(screen(), 4);
        let first_entry = area.y + 2; // past the top border and the query row
        let highlighted = |app: &AppState| -> Vec<bool> {
            let buffer = render_to_buffer(app, 100, 20);
            (0..2)
                .map(|i| row_is_highlighted(&buffer, area, first_entry + i))
                .collect()
        };

        assert_eq!(highlighted(&app), [true, false]);

        press(&mut app, KeyCode::Down);

        assert_eq!(highlighted(&app), [false, true]);
    }

    #[test]
    fn the_switcher_scrolls_to_keep_the_highlight_in_view() {
        let mut app = app_with_many_channels(30, 0);
        open_switcher(&mut app);
        for _ in 0..20 {
            press(&mut app, KeyCode::Down);
        }

        let buffer = render_to_buffer(&app, 100, 24);

        let area = switcher_area(Rect::new(0, 0, 100, 24), 30);
        assert_eq!(
            area.height,
            SWITCHER_MAX_ROWS as u16 + 3,
            "capped, then it scrolls"
        );
        let row = entry_row(&buffer, area.x, "#", "channel-20");
        assert!(row_is_highlighted(&buffer, area, row));
        let text = box_rows(&buffer, area).join("\n");
        assert!(!text.contains("channel-00"), "got:\n{text}");
        assert!(
            (area.y..area.bottom()).any(|y| buffer[(area.right() - 1, y)].symbol() == "▐"),
            "a scroll thumb on the box's right border"
        );
    }

    #[test]
    fn a_switcher_with_no_matches_says_so() {
        let mut app = app_with_every_kind();
        open_switcher(&mut app);
        type_text(&mut app, "zzz");

        let buffer = render_to_buffer(&app, 100, 20);

        let text = box_rows(&buffer, switcher_area(screen(), 0)).join("\n");
        assert!(text.contains("no matches"), "got:\n{text}");
    }

    #[test]
    fn the_cursor_sits_just_after_the_typed_query() {
        let mut app = app_with_every_kind();
        open_switcher(&mut app);
        type_text(&mut app, "ali");
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20))
            .expect("a test backend never fails");
        terminal
            .draw(|frame| render(frame, &app))
            .expect("drawing to a test backend never fails");

        let cursor = terminal
            .get_cursor_position()
            .expect("a test backend never fails");

        // Past the border, the 2-cell prompt and the 3 typed characters, on
        // the row under the top border -- not down in the input box.
        let area = switcher_area(screen(), 0);
        assert_eq!((cursor.x, cursor.y), (area.x + 1 + 2 + 3, area.y + 1));
    }

    #[test]
    fn the_switcher_covers_what_is_underneath() {
        let mut app = app_with_every_kind();
        // Long enough to run right through the box's columns.
        app.load_history("general", vec![text_message(1, [1; 32], &"X".repeat(60))]);
        open_switcher(&mut app);

        let buffer = render_to_buffer(&app, 100, 20);

        let text = box_rows(&buffer, switcher_area(screen(), 4)).join("\n");
        assert!(
            !text.contains('X'),
            "the message pane showed through:\n{text}"
        );
    }

    #[test]
    fn the_switcher_survives_a_tiny_terminal() {
        let mut app = app_with_many_channels(30, 0);
        open_switcher(&mut app);
        type_text(&mut app, "channel");

        for width in 1..=14 {
            for height in 1..=8 {
                render_to_buffer(&app, width, height);
            }
        }
    }

    #[test]
    fn the_switcher_box_is_centered_top_anchored_and_sized_to_its_matches() {
        let screen = Rect::new(0, 0, 100, 40);

        assert_eq!(
            switcher_area(screen, 0).height,
            4,
            "a row for the placeholder"
        );
        assert_eq!(switcher_area(screen, 3).height, 6);
        assert_eq!(
            switcher_area(screen, 50).height,
            SWITCHER_MAX_ROWS as u16 + 3
        );
        let area = switcher_area(screen, 3);
        assert_eq!(area.width, SWITCHER_WIDTH);
        assert_eq!(area.x * 2 + area.width, screen.width, "centered");
        assert_eq!(
            area.y, 2,
            "anchored, so it does not jump as typing shrinks it"
        );
    }

    #[test]
    fn the_switcher_box_never_outgrows_the_screen() {
        for width in 1..=120 {
            for height in 1..=40 {
                for matches in [0, 1, 5, 10, 11, 30] {
                    let screen = Rect::new(0, 0, width, height);
                    let area = switcher_area(screen, matches);
                    assert!(
                        area.right() <= screen.right() && area.bottom() <= screen.bottom(),
                        "{area:?} overflows {screen:?} for {matches} matches"
                    );
                }
            }
        }
    }

    // -- the file picker ------------------------------------------------------

    use std::path::PathBuf;

    use chrono::TimeZone;

    use crate::app::InputAction;
    use crate::picker::FolderListing;

    fn picked_file(name: &str, size: u64) -> Entry {
        Entry::new(
            name.to_string(),
            Kind::File {
                size,
                modified: SystemTime::UNIX_EPOCH,
            },
        )
    }

    fn picked_folder(name: &str) -> Entry {
        Entry::new(name.to_string(), Kind::Folder)
    }

    /// Opens the picker on `app` with a bare `/send`, in `dir`, and returns
    /// the id of the read it asked for -- unanswered, so it is still reading.
    fn open_picker_unanswered(app: &mut AppState, dir: &str) -> u64 {
        app.load_browse_dir(PathBuf::from(dir));
        type_text(app, "/send");
        match app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)) {
            Some(InputAction::ReadFolder { request, .. }) => request,
            _ => panic!("a bare /send opens the picker"),
        }
    }

    /// Opens the picker on `app` in `dir` and answers its read with
    /// `entries`.
    fn open_picker_in(app: &mut AppState, dir: &str, entries: Vec<Entry>) {
        let request = open_picker_unanswered(app, dir);
        app.apply_folder_listing(FolderListing {
            request,
            result: Ok(entries),
        });
    }

    fn open_picker(app: &mut AppState, entries: Vec<Entry>) {
        open_picker_in(app, "/pics", entries);
    }

    /// The picker's box, one string per row, borders included.
    fn picker_box(app: &AppState) -> Vec<String> {
        box_rows(&render_to_buffer(app, 100, 20), picker_area(screen()))
    }

    #[test]
    fn the_picker_is_not_drawn_until_it_is_opened() {
        let mut app = app_with_every_kind();
        let closed = render_to_rows(&app);
        assert!(!closed.iter().any(|row| row.contains("/pics")));

        open_picker(&mut app, vec![picked_file("a.txt", 1)]);

        let open = render_to_rows(&app);
        assert!(open.iter().any(|row| row.contains("/pics")), "{open:#?}");
    }

    #[test]
    fn the_picker_lists_folders_before_files_and_titles_itself_with_the_folder() {
        let mut app = app_with_every_kind();
        open_picker(
            &mut app,
            vec![
                picked_folder("docs"),
                picked_file("a.txt", 1),
                picked_file("b.txt", 1),
            ],
        );

        let rows = picker_box(&app);

        let row_of = |needle: &str| {
            rows.iter()
                .position(|row| row.contains(needle))
                .unwrap_or_else(|| panic!("no row has {needle:?}:\n{}", rows.join("\n")))
        };
        assert!(row_of("docs/") < row_of("a.txt") && row_of("a.txt") < row_of("b.txt"));
        assert!(rows[0].contains("/pics"), "got: {}", rows[0]);
    }

    #[test]
    fn a_file_row_shows_its_size_and_a_folder_row_shows_nothing_after_its_name() {
        let mut app = app_with_every_kind();
        open_picker(
            &mut app,
            vec![picked_folder("docs"), picked_file("report.pdf", 2_150_000)],
        );

        let rows = picker_box(&app);

        let file = rows.iter().find(|row| row.contains("report.pdf")).unwrap();
        assert!(file.contains("2.1 MB"), "got: {file}");
        let folder = rows.iter().find(|row| row.contains("docs/")).unwrap();
        assert_eq!(folder.trim_matches(|c| c == '│' || c == ' '), "docs/");
    }

    #[test]
    fn marked_files_carry_a_check_mark_and_the_title_counts_them() {
        let mut app = app_with_every_kind();
        open_picker(
            &mut app,
            vec![picked_file("a.txt", 1), picked_file("b.txt", 1)],
        );
        press(&mut app, KeyCode::Char(' ')); // marks a.txt

        let rows = picker_box(&app);

        let marked = rows.iter().find(|row| row.contains("a.txt")).unwrap();
        let unmarked = rows.iter().find(|row| row.contains("b.txt")).unwrap();
        assert!(marked.contains('✓'), "got: {marked}");
        assert!(!unmarked.contains('✓'), "got: {unmarked}");
        assert!(rows[0].contains("1 marked"), "got: {}", rows[0]);
    }

    #[test]
    fn the_highlight_follows_down_and_is_styled_like_the_switchers() {
        let mut app = app_with_every_kind();
        open_picker(
            &mut app,
            vec![
                picked_file("a.txt", 1),
                picked_file("b.txt", 1),
                picked_file("c.txt", 1),
            ],
        );
        let area = picker_area(screen());
        let first_entry = area.y + 2; // past the top border and the filter row
        let highlighted = |app: &AppState| -> Vec<bool> {
            let buffer = render_to_buffer(app, 100, 20);
            (0..2)
                .map(|i| row_is_highlighted(&buffer, area, first_entry + i))
                .collect()
        };

        assert_eq!(highlighted(&app), [true, false]);

        press(&mut app, KeyCode::Down);

        assert_eq!(highlighted(&app), [false, true]);
    }

    #[test]
    fn a_marked_row_that_is_highlighted_keeps_the_highlight_colors() {
        let mut app = app_with_every_kind();
        open_picker(&mut app, vec![picked_file("a.txt", 1)]);
        press(&mut app, KeyCode::Char(' '));

        let buffer = render_to_buffer(&app, 100, 20);

        let area = picker_area(screen());
        let row = area.y + 2;
        // The check mark sits in the gutter's middle cell.
        let mark = &buffer[(area.x + 1 + 1, row)];
        assert_eq!(mark.symbol(), "✓");
        assert_eq!((mark.fg, mark.bg), (Color::Black, Color::Cyan));
    }

    #[test]
    fn typing_narrows_the_list_and_the_cursor_sits_after_the_filter() {
        let mut app = app_with_every_kind();
        open_picker(
            &mut app,
            vec![
                picked_folder("docs"),
                picked_file("notes.txt", 1),
                picked_file("a.txt", 1),
            ],
        );
        type_text(&mut app, "A.T");
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 20))
            .expect("a test backend never fails");
        terminal
            .draw(|frame| render(frame, &app))
            .expect("drawing to a test backend never fails");

        let text = box_rows(terminal.backend().buffer(), picker_area(screen())).join("\n");
        let cursor = terminal
            .get_cursor_position()
            .expect("a test backend never fails");

        assert!(text.contains("a.txt"), "got:\n{text}");
        assert!(
            !text.contains("notes.txt") && !text.contains("docs/"),
            "got:\n{text}"
        );
        // Past the border, the 2-cell prompt and the 3 typed characters, on
        // the row under the top border -- not down in the input box.
        let area = picker_area(screen());
        assert_eq!((cursor.x, cursor.y), (area.x + 1 + 2 + 3, area.y + 1));
    }

    #[test]
    fn the_picker_scrolls_to_keep_the_highlight_in_view() {
        let mut app = app_with_every_kind();
        let files = (0..40)
            .map(|i| picked_file(&format!("file-{i:02}.txt"), 1))
            .collect();
        open_picker(&mut app, files);
        for _ in 0..30 {
            press(&mut app, KeyCode::Down);
        }

        let buffer = render_to_buffer(&app, 100, 20);

        let area = picker_area(screen());
        let rows = box_rows(&buffer, area);
        let row = (area.y..area.bottom())
            .find(|&y| rows[(y - area.y) as usize].contains("file-30.txt"))
            .expect("the highlighted file is in view");
        assert!(row_is_highlighted(&buffer, area, row));
        assert!(!rows.join("\n").contains("file-00.txt"));
        assert!(
            (area.y..area.bottom()).any(|y| buffer[(area.right() - 1, y)].symbol() == "▐"),
            "a scroll thumb on the box's right border"
        );
    }

    #[test]
    fn a_folder_still_being_read_says_so() {
        let mut app = app_with_every_kind();
        open_picker_unanswered(&mut app, "/pics");

        let rows = picker_box(&app);

        assert!(rows[2].contains("reading..."), "got:\n{}", rows.join("\n"));
    }

    #[test]
    fn an_empty_folder_says_so() {
        let mut app = app_with_every_kind();
        open_picker(&mut app, Vec::new());

        let rows = picker_box(&app);

        assert!(
            rows[2].contains("empty folder"),
            "got:\n{}",
            rows.join("\n")
        );
    }

    #[test]
    fn a_filter_that_matches_nothing_says_so() {
        let mut app = app_with_every_kind();
        open_picker(&mut app, vec![picked_file("a.txt", 1)]);
        type_text(&mut app, "zzz");

        let rows = picker_box(&app);

        assert!(rows[2].contains("no matches"), "got:\n{}", rows.join("\n"));
    }

    #[test]
    fn a_notice_takes_the_place_of_the_key_hints() {
        let mut app = app_with_every_kind();
        open_picker(&mut app, vec![picked_folder("docs")]);
        let hints = picker_box(&app);
        assert!(hints.last().unwrap().contains("Esc cancel"), "{hints:#?}");

        let Some(InputAction::ReadFolder { request, .. }) =
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("Enter on a folder reads it");
        };
        app.apply_folder_listing(FolderListing {
            request,
            result: Err("can't open /pics/docs: Permission denied".to_string()),
        });

        let rows = picker_box(&app);
        let bottom = rows.last().unwrap();
        assert!(bottom.contains("can't open /pics/docs"), "got: {bottom}");
        assert!(!bottom.contains("Esc cancel"), "got: {bottom}");
        assert!(
            rows.iter().any(|row| row.contains("docs/")),
            "the old listing is still there:\n{}",
            rows.join("\n")
        );
    }

    #[test]
    fn a_notice_too_long_for_the_row_loses_its_start_and_keeps_the_reason() {
        let mut app = app_with_every_kind();
        open_picker(&mut app, vec![picked_folder("docs")]);
        let Some(InputAction::ReadFolder { request, .. }) =
            app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        else {
            panic!("Enter on a folder reads it");
        };
        app.apply_folder_listing(FolderListing {
            request,
            result: Err(format!(
                "can't open {}: Permission denied (os error 13)",
                "x".repeat(120)
            )),
        });

        let rows = picker_box(&app);

        let bottom = rows.last().unwrap();
        assert!(bottom.contains('…'), "got: {bottom}");
        assert!(
            bottom.contains("Permission denied (os error 13)"),
            "the reason must survive: {bottom}"
        );
    }

    #[test]
    fn a_long_name_is_cut_short_of_the_size_column() {
        let mut app = app_with_every_kind();
        let long =
            "a-file-name-that-is-far-too-long-to-fit-in-the-picker-next-to-its-size-and-date.txt";
        open_picker(&mut app, vec![picked_file(long, 480)]);

        let rows = picker_box(&app);

        let row = rows.iter().find(|row| row.contains("a-file-name")).unwrap();
        assert!(row.contains('…'), "got: {row}");
        assert!(!row.contains("date.txt"), "got: {row}");
        assert!(row.contains("480 B"), "the size always stays: {row}");
    }

    #[test]
    fn a_long_folder_path_keeps_its_tail_in_the_title() {
        let mut app = app_with_every_kind();
        let deep = format!("/{}/pictures", "very-deep/".repeat(12));
        open_picker_in(&mut app, &deep, vec![picked_file("a.txt", 1)]);

        let rows = picker_box(&app);

        assert!(rows[0].contains('…'), "got: {}", rows[0]);
        assert!(rows[0].contains("/pictures"), "got: {}", rows[0]);
    }

    #[test]
    fn the_picker_covers_what_is_underneath() {
        let mut app = app_with_every_kind();
        // Long enough to run right through the box's columns.
        app.load_history("general", vec![text_message(1, [1; 32], &"X".repeat(60))]);
        open_picker(&mut app, vec![picked_file("a.txt", 1)]);

        let text = picker_box(&app).join("\n");

        assert!(
            !text.contains('X'),
            "the message pane showed through:\n{text}"
        );
    }

    #[test]
    fn the_picker_survives_a_tiny_terminal() {
        let mut app = app_with_every_kind();
        let files = (0..40)
            .map(|i| picked_file(&format!("file-{i:02}.txt"), 1))
            .collect();
        open_picker(&mut app, files);
        press(&mut app, KeyCode::Char(' '));
        type_text(&mut app, "file");

        for width in 1..=30 {
            for height in 1..=8 {
                render_to_buffer(&app, width, height);
            }
        }
    }

    #[test]
    fn the_picker_box_is_centered_top_anchored_and_never_outgrows_the_screen() {
        let screen = Rect::new(0, 0, 120, 40);
        let area = picker_area(screen);
        assert_eq!(area.width, PICKER_WIDTH);
        assert_eq!(area.height, PICKER_ROWS as u16 + 3);
        assert_eq!(area.x * 2 + area.width, screen.width, "centered");
        assert_eq!(area.y, 2, "anchored, so it never jumps while typing");

        for width in 1..=120 {
            for height in 1..=40 {
                let screen = Rect::new(0, 0, width, height);
                let area = picker_area(screen);
                assert!(
                    area.right() <= screen.right() && area.bottom() <= screen.bottom(),
                    "{area:?} overflows {screen:?}"
                );
            }
        }
    }

    #[test]
    fn a_recent_file_shows_its_day_and_time_and_an_old_one_its_day_and_year() {
        let now = Local.with_ymd_and_hms(2026, 10, 12, 15, 0, 0).unwrap();
        let at = |year, month, day, hour, minute| {
            SystemTime::from(
                Local
                    .with_ymd_and_hms(year, month, day, hour, minute, 0)
                    .unwrap(),
            )
        };

        let today = modified_label(at(2026, 10, 12, 14, 3), now);
        let months_ago = modified_label(at(2026, 5, 20, 9, 30), now);
        let last_year = modified_label(at(2025, 3, 3, 11, 20), now);

        assert_eq!(today, "12 Oct 14:03");
        assert_eq!(months_ago, "20 May 09:30");
        assert_eq!(last_year, "03 Mar  2025");
        for label in [today, months_ago, last_year] {
            assert_eq!(label.chars().count(), PICKER_TIME_WIDTH, "{label}");
        }
    }

    #[test]
    fn truncate_start_keeps_the_end_of_a_long_path() {
        assert_eq!(truncate_start("/short", 10), "/short");
        assert_eq!(truncate_start("/Users/jurgen/Downloads", 10), "…Downloads");
        assert_eq!(truncate_start("/anything", 0), "");
    }

    #[test]
    fn a_renamed_group_channel_shows_its_local_name_in_the_panel_and_the_pane_title() {
        let mut app = app_with_every_kind();
        app.active = 2; // project-x
        type_text(&mut app, "/rename proj");
        press(&mut app, KeyCode::Enter);

        let buffer = render_to_buffer(&app, 100, 20);

        nav_row(&buffer, "#", "proj");
        let panel = nav_rows(&buffer).join("\n");
        assert!(
            !panel.contains("project-x"),
            "the stored name leaked:\n{panel}"
        );
        let rows = screen_rows(&buffer);
        assert!(
            rows.iter().any(|row| row.contains("#proj · 0 peer(s)")),
            "got: {rows:#?}"
        );
    }
}
