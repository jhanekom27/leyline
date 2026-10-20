use super::*;

pub(super) fn render_messages(frame: &mut Frame, area: Rect, app: &AppState) {
    let channel = app.active();
    let heading = app.channel_heading(&channel.name);
    let visible_rows = area.height.saturating_sub(2).max(1) as usize; // minus borders
    let inner_width = (area.width as usize).saturating_sub(2).max(1); // minus borders

    // Flattened one-`Line`-per-terminal-row, not one per `TranscriptLine`
    // -- a wrapped message now spans several rows, so the scroll window
    // below has to work in rows to stay accurate. `picked_row` is the
    // header row of whichever message matches `app.picking_reply`, if
    // any -- see the window adjustment below.
    let (mut rows, picked_row, unread_row) = build_message_rows(app, channel, inner_width);
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
    if let Some(unread) = unread_row {
        start = unread;
        end = (start + visible_rows).min(total);
    }
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
pub(super) fn render_scroll_indicator(
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
pub(super) fn build_message_rows(
    app: &AppState,
    channel: &Channel,
    inner_width: usize,
) -> (Vec<Line<'static>>, Option<usize>, Option<usize>) {
    if let Some(thread) = &channel.thread {
        let (rows, picked_row) = build_thread_rows(app, thread, inner_width);
        return (rows, picked_row, None);
    }
    if let Some(search) = &channel.search {
        let (rows, picked_row) = build_search_rows(app, search, inner_width);
        return (rows, picked_row, None);
    }

    let layout = row_layout(inner_width);
    let mut rows = Vec::new();
    let mut picked_row = None;
    let mut unread_row = None;
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
                if channel.unread_anchor == Some(message.id) {
                    unread_row = Some(rows.len());
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
                // unreadable/uncopyable. A multi-line notice (e.g. `/help`)
                // gets one row per `\n`, its continuation lines indented
                // under the first line's `* ` marker.
                let style = Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC);
                for (index, line) in text.split('\n').enumerate() {
                    let prefix = if index == 0 { "* " } else { "  " };
                    for chunk in markdown::wrap_chars(&format!("{prefix}{line}"), inner_width) {
                        rows.push(Line::from(Span::styled(chunk, style)));
                    }
                }
            }
        }
    }
    (rows, picked_row, unread_row)
}

/// Renders a channel's active `/search` results in place of its normal
/// transcript -- reuses the same per-message formatting
/// (`push_chat_rows`), grouped the same way, but with no `System` lines to
/// interleave since a search snapshot only ever holds chat matches.
/// Highlights the matched substring in each line (see `highlight_spans`).
pub(super) fn build_search_rows(
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
pub(super) fn build_thread_rows(
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
pub(super) struct RowLayout {
    wrap_width: usize,
    blank_time: String,
    blank_name: String,
}

pub(super) fn row_layout(inner_width: usize) -> RowLayout {
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
pub(super) fn push_chat_rows(
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
pub(super) fn attachment_line(app: &AppState, message: &ChatMessage) -> Option<String> {
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
pub(super) fn find_highlights(text: &str, term_lower: &str) -> Vec<(String, bool)> {
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
pub(super) fn highlighted_spans(text: &str, term_lower: Option<&str>) -> Vec<Span<'static>> {
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
pub(super) fn highlight_spans(
    spans: Vec<Span<'static>>,
    term_lower: Option<&str>,
) -> Vec<Span<'static>> {
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
pub(super) fn reply_preview_spans(app: &AppState, reply_to: u64) -> Vec<Span<'static>> {
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
pub(super) fn reply_snippet(message: &ChatMessage) -> String {
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
pub(super) fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{truncated}…")
}

/// Truncates `text` to at most `max_chars` characters by dropping its
/// start, prefixing `…` if it was longer -- for a path, whose end is the
/// part that tells you where you are.
pub(super) fn truncate_start(text: &str, max_chars: usize) -> String {
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
pub(super) fn reversed_span(span: Span<'static>) -> Span<'static> {
    Span {
        style: span.style.add_modifier(Modifier::REVERSED),
        ..span
    }
}

/// Pads `name` to exactly `width` characters, truncating with a trailing
/// `…` if it's longer, so the `│` separator after the name column always
/// lines up regardless of how long a nickname or petname is.
pub(super) fn fit_name(name: &str, width: usize) -> String {
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
pub(super) fn format_time(ts_unix_ms: u64) -> String {
    match DateTime::from_timestamp_millis(ts_unix_ms as i64) {
        Some(dt) => dt.with_timezone(&Local).format("%H:%M").to_string(),
        None => "--:--".to_string(),
    }
}

/// Like `format_time`, but followed by the local date -- for a peer's
/// popup, where "last seen" may well be days ago.
pub(super) fn format_date_time(ts_unix_ms: u64) -> String {
    match DateTime::from_timestamp_millis(ts_unix_ms as i64) {
        Some(dt) => dt
            .with_timezone(&Local)
            .format("%H:%M %Y-%m-%d")
            .to_string(),
        None => "--:-- ----------".to_string(),
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
pub(super) fn user_color(id: &[u8; 32]) -> Color {
    let hash = id.iter().fold(0u32, |acc, &byte| {
        acc.wrapping_mul(31).wrapping_add(byte as u32)
    });
    USER_PALETTE[hash as usize % USER_PALETTE.len()]
}
