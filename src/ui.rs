//! Pure rendering: reads `AppState`, writes widgets to the frame. No
//! mutation, no I/O -- see concept.md's "one owner of UI state" principle.

use chrono::{DateTime, Local};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::scrollbar;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
};
use textwrap::wrap;

use crate::app::{AppState, Channel, TranscriptLine, hex_id};
use crate::files::human_size;
use crate::markdown;
use crate::message::ChatMessage;
use crate::search::SearchResults;

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

/// Short usage reminders for every slash command, shown in the sidebar's
/// commands panel (`render_hints`) when `AppState::show_hints` is on.
/// Kept in sync with the Commands table in README.md and `run_help`'s
/// system notice.
const COMMAND_HINTS: &[&str] = &[
    "/join <name|ticket>",
    "/invite",
    "/leave [channel]",
    "/who",
    "/send <path>",
    "/save <hash>",
    "/paste (or Ctrl+V)",
    "/alias <hex> <name>",
    "/nick <name>",
    "/search <term> (/s)",
    "/reply [text]",
    "/hints",
    "/bell",
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
    let [header, body, input] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(input_box_height(&app.input)),
    ])
    .areas(frame.area());

    render_header(frame, header, app);
    render_body(frame, body, app);
    render_input(frame, input, app);
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
    let [messages, sidebar] =
        Layout::horizontal([Constraint::Min(20), Constraint::Length(22)]).areas(area);

    render_messages(frame, messages, app);
    render_sidebar(frame, sidebar, app);
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
    } else {
        match &channel.search {
            Some(search) => {
                let status = if search.pending {
                    ", searching full history..."
                } else {
                    ""
                };
                format!(
                    "#{} · search '{}' ({}{status}) · /s to clear",
                    channel.name,
                    search.term,
                    search.messages.len()
                )
            }
            None => format!("#{} · {} peer(s)", channel.name, channel.peers.len()),
        }
    };
    let list = List::new(items).block(
        Block::bordered()
            .title(title)
            .border_type(BorderType::Rounded),
    );
    frame.render_widget(list, area);
    render_scroll_indicator(frame, area, start, total, visible_rows);
}

/// Overlays a scroll-position thumb on the messages block's right border,
/// so it's clear at a glance how far back you've scrolled and how much
/// history is above. `start` is the same top-of-viewport row index used
/// to slice the visible window above. Only drawn once the transcript
/// actually overflows one screen (`total > visible_rows`) -- a
/// full-height thumb on a short conversation would just be noise.
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
/// Delegates to `build_search_rows` when `channel` has an active `/search`
/// (see `app::Channel::search`), rendering its results in place of the
/// normal transcript.
fn build_message_rows(
    app: &AppState,
    channel: &Channel,
    inner_width: usize,
) -> (Vec<Line<'static>>, Option<usize>) {
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
                let is_picked = app.picking_reply == Some(message.id);
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
        let is_picked = app.picking_reply == Some(message.id);
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

    // A synthesized attachment caption (e.g. a filename) is plain text,
    // never markdown -- only an actual message body goes through
    // `markdown::render`.
    let caption = attachment_caption(app, message);
    let mut bodies: Vec<Vec<Span<'static>>> = match &caption {
        Some(caption) => {
            let mut wrapped = wrap(caption, layout.wrap_width);
            if wrapped.is_empty() {
                wrapped.push(std::borrow::Cow::Borrowed(""));
            }
            wrapped
                .iter()
                .map(|chunk| highlighted_spans(chunk, term_lower))
                .collect()
        }
        None => markdown::render(&message.text, layout.wrap_width)
            .into_iter()
            .map(|row| highlight_spans(row, term_lower))
            .collect(),
    };

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
/// since the file is already local. `None` for a plain text message, in
/// which case `push_chat_rows` falls back to `message.text` unchanged.
fn attachment_caption(app: &AppState, message: &ChatMessage) -> Option<String> {
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
    let mut items: Vec<ListItem> = channel.peers.iter().map(|id| peer_item(app, id)).collect();
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
    let lines: Vec<&str> = app.input.split('\n').collect();
    let (cursor_line, cursor_col) = cursor_line_and_col(&app.input, app.cursor);

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
    fn attachment_caption_includes_a_save_hint_for_someone_elses_file() {
        let app = AppState::new([1; 32], vec!["general".to_string()], "general");
        let message = file_message([2; 32], "report.pdf", 2_150_000);

        let caption = attachment_caption(&app, &message).unwrap();

        assert!(caption.contains("report.pdf"), "got: {caption}");
        assert!(caption.contains("2.1 MB"), "got: {caption}");
        assert!(caption.contains("/save"), "got: {caption}");
    }

    #[test]
    fn attachment_caption_omits_the_save_hint_for_your_own_file() {
        let self_id = [1; 32];
        let app = AppState::new(self_id, vec!["general".to_string()], "general");
        let message = file_message(self_id, "report.pdf", 10);

        let caption = attachment_caption(&app, &message).unwrap();

        assert!(caption.contains("report.pdf"));
        assert!(!caption.contains("/save"), "got: {caption}");
    }

    #[test]
    fn attachment_caption_is_none_for_a_plain_text_message() {
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

        assert_eq!(attachment_caption(&app, &message), None);
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
}
