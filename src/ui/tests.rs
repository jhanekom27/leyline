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
#[test]
fn build_message_rows_returns_the_first_unread_messages_row_as_the_viewport_anchor() {
    let mut app = AppState::new([9; 32], vec!["general".to_string()], "general");
    app.load_history(
        "general",
        vec![
            text_message(1, [1; 32], "already read"),
            text_message(2, [1; 32], "first unread"),
            text_message(3, [1; 32], "also unread"),
        ],
    );
    app.active_mut().unread_anchor = Some(2);

    let (_, _, unread_row) = build_message_rows(&app, app.active(), 80);

    assert_eq!(unread_row, Some(1));
}

#[test]
fn build_message_rows_gives_each_line_of_a_system_notice_its_own_row() {
    let mut app = AppState::new([9; 32], vec!["general".to_string()], "general");
    app.push_system("first\n\nsecond");

    let (rows, _, _) = build_message_rows(&app, app.active(), 80);
    let texts: Vec<String> = rows.iter().map(|row| row.to_string()).collect();

    assert_eq!(texts, vec!["* first", "  ", "  second"]);
}

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;

/// Renders `app` into an off-screen terminal of `width` x `height`
/// cells and returns what was drawn, so a test can assert on what a
/// user would actually see rather than on the state that drives it.
fn render_to_buffer(app: &AppState, width: u16, height: u16) -> Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
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
fn an_unread_channel_ends_in_its_count_but_the_active_one_never_does() {
    let mut app = app_with_every_kind();
    app.channels[2].unread_count = 12; // project-x, inactive
    app.channels[0].unread_count = 1; // general, active: the count is moot

    let buffer = render_to_buffer(&app, 100, 20);

    let unread = nav_row(&buffer, "#", "project-x");
    let active = nav_row(&buffer, "#", "general");
    let read = nav_row(&buffer, "@", "07070707");
    assert_eq!(buffer[(NAV_MARKER_X - 1, unread)].symbol(), "1");
    assert_eq!(buffer[(NAV_MARKER_X, unread)].symbol(), "2");
    assert_eq!(buffer[(NAV_MARKER_X, active)].symbol(), " ");
    assert_eq!(buffer[(NAV_MARKER_X, read)].symbol(), " ");
}

#[test]
fn a_long_label_is_cut_short_of_the_unread_count() {
    let long = "a-channel-name-far-too-long-for-the-panel";
    let mut app = AppState::new(
        [9; 32],
        vec!["general".to_string(), long.to_string()],
        "general",
    );
    app.channels[1].unread_count = 12;

    let buffer = render_to_buffer(&app, 100, 20);

    let row = nav_row(&buffer, "#", "a-channel-name…");
    assert_eq!(buffer[(NAV_MARKER_X - 2, row)].symbol(), " ");
    assert_eq!(buffer[(NAV_MARKER_X - 1, row)].symbol(), "1");
    assert_eq!(buffer[(NAV_MARKER_X, row)].symbol(), "2");
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

#[test]
fn peers_sidebar_distinguishes_online_and_last_seen_peers() {
    let mut app = app_with_every_kind();
    app.load_contacts(HashMap::from([
        ([1; 32], "Alice".to_string()),
        ([2; 32], "Bob".to_string()),
    ]));
    app.active_mut().peers.push([1; 32]);
    app.record_peer_seen("general", [1; 32], 1_000);
    app.record_peer_seen("general", [2; 32], 2_000);

    let rows = screen_rows(&render_to_buffer(&app, 100, 50));
    let text = rows.join("\n");

    assert!(text.contains("● Alice"), "got: {text}");
    assert!(text.contains("○ Bob"), "got: {text}");
    assert!(!text.contains("● Bob"), "got: {text}");
    assert!(!text.contains("last seen"), "got: {text}");
    // Online rows (ours included) all come before offline ones.
    let row_of = |needle: &str| rows.iter().position(|row| row.contains(needle)).unwrap();
    assert!(row_of("● Alice") < row_of("● you"), "got: {text}");
    assert!(row_of("● you") < row_of("○ Bob"), "got: {text}");
}

#[test]
fn peers_sidebar_only_shows_active_channels_last_seen_peers() {
    let mut app = app_with_every_kind();
    app.load_contacts(HashMap::from([
        ([1; 32], "Alice".to_string()),
        ([2; 32], "Bob".to_string()),
    ]));
    app.record_peer_seen("general", [1; 32], 1_000);
    app.record_peer_seen("project-x", [2; 32], 2_000);

    let general = screen_rows(&render_to_buffer(&app, 100, 50)).join("\n");
    assert!(general.contains("○ Alice"), "got: {general}");
    assert!(!general.contains("○ Bob"), "got: {general}");

    app.active = 2;
    let project = screen_rows(&render_to_buffer(&app, 100, 50)).join("\n");
    assert!(project.contains("○ Bob"), "got: {project}");
    assert!(!project.contains("○ Alice"), "got: {project}");
}

/// The screen every switcher render test draws on.
fn screen() -> Rect {
    Rect::new(0, 0, 100, 20)
}

fn open_switcher(app: &mut AppState) {
    app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
}
fn open_command_picker(app: &mut AppState) {
    app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
}
fn open_peers_overlay(app: &mut AppState) {
    type_text(app, "/who");
    press(app, KeyCode::Enter);
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
fn command_picker_shows_command_syntax_and_descriptions() {
    let mut app = app_with_every_kind();
    open_command_picker(&mut app);

    let text = render_to_rows(&app).join("\n");

    assert!(text.contains("commands"), "got: {text}");
    assert!(text.contains("/join <name|ticket>"), "got: {text}");
    assert!(text.contains("Join or create a channel"), "got: {text}");
}

#[test]
fn command_picker_shows_shortcuts_in_a_dedicated_column() {
    let mut app = app_with_every_kind();
    open_command_picker(&mut app);
    type_text(&mut app, "paste");
    let rows = render_to_rows(&app);
    let text = rows.join("\n");

    assert!(text.contains("commands · Ctrl+K"), "got: {text}");
    assert!(text.contains("Ctrl+P channels"), "got: {text}");
    assert!(text.contains("/paste"), "got: {text}");
    assert!(text.contains("Ctrl+V"), "got: {text}");
    assert!(text.contains("Share the clipboard"), "got: {text}");
    let header = rows.iter().find(|row| row.contains("shortcut")).unwrap();
    let paste = rows.iter().find(|row| row.contains("Ctrl+V")).unwrap();
    let column = |row: &str, needle: &str| {
        row.find(needle)
            .map(|index| row[..index].chars().count())
            .unwrap()
    };
    assert_eq!(
        column(header, "shortcut"),
        column(paste, "Ctrl+V"),
        "shortcut labels must align under their dedicated header"
    );

    press(&mut app, KeyCode::Esc);
    open_command_picker(&mut app);
    type_text(&mut app, "invite");
    let text = render_to_rows(&app).join("\n");
    assert!(text.contains("/invite"), "got: {text}");
    assert!(!text.contains("[Ctrl+V]"), "got: {text}");
}

#[test]
fn command_picker_shows_each_selection_outcome_before_enter() {
    let mut app = app_with_every_kind();
    open_command_picker(&mut app);
    type_text(&mut app, "files");
    let rows = render_to_rows(&app);
    let text = rows.join("\n");

    assert!(text.contains("Enter run · draft kept"), "got: {text}");
    let header = rows.iter().find(|row| row.contains("action")).unwrap();
    let files = rows.iter().find(|row| row.contains("/files")).unwrap();
    assert!(files.contains("▶ run"), "got: {files}");
    assert_eq!(
        header.find("action"),
        files.find("▶ run"),
        "action labels must align under their dedicated header"
    );

    press(&mut app, KeyCode::Esc);
    open_command_picker(&mut app);
    type_text(&mut app, "search");
    let text = render_to_rows(&app).join("\n");
    assert!(text.contains("✎ edit"), "got: {text}");
    assert!(
        text.contains("Enter insert · replaces draft"),
        "got: {text}"
    );
}

#[test]
fn command_picker_shows_reply_and_thread_shortcuts() {
    let mut app = app_with_every_kind();
    open_command_picker(&mut app);
    type_text(&mut app, "reply");
    let reply = render_to_rows(&app).join("\n");
    assert!(reply.contains("Ctrl+R"), "got: {reply}");

    press(&mut app, KeyCode::Esc);
    open_command_picker(&mut app);
    type_text(&mut app, "thread");
    let thread = render_to_rows(&app).join("\n");
    assert!(thread.contains("Ctrl+T"), "got: {thread}");
}

#[test]
fn command_picker_shortcuts_survive_narrow_terminals() {
    let mut app = app_with_every_kind();
    open_command_picker(&mut app);
    type_text(&mut app, "paste");

    for width in 1..=24 {
        for height in 1..=8 {
            render_to_buffer(&app, width, height);
        }
    }

    let buffer = render_to_buffer(&app, 24, 8);
    let area = command_picker_area(Rect::new(0, 0, 24, 8), 1);
    let text = box_rows(&buffer, area).join("\n");
    assert!(text.contains("Ctrl+V"), "got: {text}");
    assert!(text.contains("▶ run"), "got: {text}");
}
#[test]
fn command_picker_uses_more_rows_and_keeps_selected_text_readable() {
    let mut app = app_with_every_kind();
    open_command_picker(&mut app);

    let screen = Rect::new(0, 0, 100, 40);
    let area = command_picker_area(screen, app.command_picker_matches().len());
    assert_eq!(area.height, COMMAND_PICKER_ROWS as u16 + 4);

    let buffer = render_to_buffer(&app, screen.width, screen.height);
    let selected_syntax = &buffer[(area.x + 2, area.y + 3)];
    assert_eq!(selected_syntax.fg, Color::Black);
    assert_eq!(selected_syntax.bg, Color::Cyan);
}

#[test]
fn who_overlay_shows_endpoint_alias_and_nickname_in_aligned_columns() {
    use crate::message::IdentityAnnounce;
    use crate::net::NetEvent;

    let alice = [7; 32];
    let unnamed = [8; 32];
    let mut app = app_with_every_kind();
    app.active_mut().peers = vec![alice];
    app.record_peer_seen("general", unnamed, 100);
    app.load_settings(true, true, Some("me".to_string()));
    app.load_contacts(HashMap::from([(alice, "Alice".to_string())]));
    app.handle_net_event(NetEvent::Identity(
        "general".to_string(),
        IdentityAnnounce {
            sender: alice,
            nickname: "ally".to_string(),
        },
    ));
    open_peers_overlay(&mut app);

    let screen = Rect::new(0, 0, 120, 30);
    let buffer = render_to_buffer(&app, screen.width, screen.height);
    let area = peers_overlay_area(screen, 3);
    let rows = box_rows(&buffer, area);
    let header = rows.iter().find(|row| row.contains("endpoint id")).unwrap();
    let alice_row = rows
        .iter()
        .find(|row| row.contains(&hex_id(&alice)))
        .unwrap();
    let unnamed_row = rows
        .iter()
        .find(|row| row.contains(&hex_id(&unnamed)))
        .unwrap();
    let self_row = rows
        .iter()
        .find(|row| row.contains(&hex_id(&app.self_id)))
        .unwrap();
    let column = |row: &str, needle: &str| row[..row.find(needle).unwrap()].chars().count();

    assert!(rows.iter().any(|row| row.contains("peers in #general")));
    assert!(rows.iter().any(|row| row.contains("Esc close")));
    assert_eq!(
        column(header, "endpoint id"),
        column(alice_row, &hex_id(&alice))
    );
    assert_eq!(column(header, "alias"), column(alice_row, "Alice"));
    assert_eq!(column(header, "nickname"), column(alice_row, "ally"));
    assert!(alice_row.contains("● "), "online row: {alice_row}");
    assert!(self_row.contains("● you "), "local row: {self_row}");
    assert!(self_row.contains("me"), "local nickname: {self_row}");
    assert!(unnamed_row.contains("○ "), "offline row: {unnamed_row}");
    assert!(
        rows.iter().position(|row| row == alice_row) < rows.iter().position(|row| row == self_row)
            && rows.iter().position(|row| row == self_row)
                < rows.iter().position(|row| row == unnamed_row),
        "online peers, this device, then remembered offline peers"
    );
    assert!(
        unnamed_row.matches('—').count() >= 2,
        "missing values use explicit table cells: {unnamed_row}"
    );
}

#[test]
fn who_overlay_includes_this_device_and_survives_tiny_terminals() {
    let mut app = app_with_every_kind();
    open_peers_overlay(&mut app);

    let screen = Rect::new(0, 0, 100, 20);
    let buffer = render_to_buffer(&app, screen.width, screen.height);
    let text = box_rows(&buffer, peers_overlay_area(screen, 1)).join("\n");
    assert!(text.contains(&hex_id(&app.self_id)), "got: {text}");
    assert!(text.contains("● you "), "got: {text}");

    for width in 1..=30 {
        for height in 1..=8 {
            render_to_buffer(&app, width, height);
        }
    }
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
fn an_unread_channel_keeps_its_count_in_the_switcher() {
    let mut app = app_with_every_kind();
    app.channels[2].unread_count = 12; // project-x
    open_switcher(&mut app);

    let buffer = render_to_buffer(&app, 100, 20);

    let area = switcher_area(screen(), 4);
    let unread = entry_row(&buffer, area.x, "#", "project-x");
    let read = entry_row(&buffer, area.x, "@", "07070707");
    assert_eq!(buffer[(area.right() - 3, unread)].symbol(), "1");
    assert_eq!(buffer[(area.right() - 2, unread)].symbol(), "2");
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

// -- the files overlay ----------------------------------------------------

/// A shared file's chat message: `file_message` with an id of its own (a
/// channel dedupes by id) and a timestamp.
fn shared_file(id: u64, ts_unix_ms: u64, sender: [u8; 32], name: &str, size: u64) -> ChatMessage {
    ChatMessage {
        id,
        ts_unix_ms,
        ..file_message(sender, name, size)
    }
}

/// Opens the files overlay on `app`'s active channel with `/files`.
fn open_files(app: &mut AppState) {
    type_text(app, "/files");
    press(app, KeyCode::Enter);
}

/// The files overlay's box on a 110x24 screen, one string per row,
/// borders included.
fn files_box(app: &AppState) -> Vec<String> {
    let screen = Rect::new(0, 0, 110, 24);
    box_rows(&render_to_buffer(app, 110, 24), files_area(screen))
}

/// The row of `rows` with `needle` in it -- panicking, with them all, if
/// there is none.
fn row_with<'a>(rows: &'a [String], needle: &str) -> &'a String {
    rows.iter()
        .find(|row| row.contains(needle))
        .unwrap_or_else(|| panic!("no row has {needle:?}:\n{}", rows.join("\n")))
}

/// `general` with three files, the newest -- `notes.txt` -- from
/// ourselves, then `map.png` from Bob and, oldest, `report.pdf` from
/// Alice. The peer behind the DM and the `/dm` chat is Alice.
fn app_with_shared_files() -> AppState {
    let mut app = app_with_every_kind();
    app.load_contacts(HashMap::from([
        ([7; 32], "Alice".to_string()),
        ([8; 32], "Bob".to_string()),
    ]));
    let now = crate::message::now_unix_ms();
    app.load_history(
        "general",
        vec![
            shared_file(1, now - 3_000, [7; 32], "report.pdf", 2_150_000),
            shared_file(2, now - 2_000, [8; 32], "map.png", 480),
            shared_file(3, now - 1_000, [9; 32], "notes.txt", 12),
        ],
    );
    app
}

/// Tells the open overlay that the file named `name` is saved.
fn mark_saved(app: &mut AppState, name: &str) {
    app.apply_saved_files(
        "general",
        HashMap::from([(
            iroh_blobs::Hash::new(name.as_bytes()),
            PathBuf::from(format!("/files/general/{name}")),
        )]),
    );
}

#[test]
fn the_files_overlay_is_not_drawn_until_it_is_opened() {
    let mut app = app_with_shared_files();
    let closed = render_to_rows(&app);
    assert!(!closed.iter().any(|row| row.contains("files in")));

    open_files(&mut app);

    let open = render_to_rows(&app);
    assert!(
        open.iter().any(|row| row.contains("files in #general")),
        "{open:#?}"
    );
}

#[test]
fn each_file_is_a_row_with_its_name_size_sender_and_date_newest_first() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    let when = |name: &str| {
        let entries = app.files_matches();
        let shared = entries
            .iter()
            .find_map(|entry| match entry {
                FilesEntry::Shared(shared) if shared.file.filename == name => Some(shared),
                _ => None,
            })
            .unwrap();
        sent_label(shared.ts_unix_ms, Local::now())
    };

    let rows = files_box(&app);

    let report = row_with(&rows, "report.pdf");
    for part in ["2.1 MB", "Alice", &when("report.pdf")] {
        assert!(report.contains(part), "{part:?} missing from: {report}");
    }
    let map = row_with(&rows, "map.png");
    for part in ["480 B", "Bob", &when("map.png")] {
        assert!(map.contains(part), "{part:?} missing from: {map}");
    }
    let notes = row_with(&rows, "notes.txt");
    for part in ["12 B", "you", &when("notes.txt")] {
        assert!(notes.contains(part), "{part:?} missing from: {notes}");
    }
    let position = |needle: &str| rows.iter().position(|row| row.contains(needle)).unwrap();
    assert!(
        position("notes.txt") < position("map.png") && position("map.png") < position("report.pdf"),
        "newest first:\n{}",
        rows.join("\n")
    );
}

#[test]
fn the_columns_line_up_from_row_to_row() {
    let mut app = app_with_shared_files();
    open_files(&mut app);

    let rows = files_box(&app);

    // The sender column starts where the size column ends, however long
    // the name before it is.
    let column = |name: &str, sender: &str| {
        let row = row_with(&rows, name);
        let at = row.find(sender).unwrap();
        row[..at].chars().count()
    };
    assert_eq!(column("report.pdf", "Alice"), column("map.png", "Bob"));
    assert_eq!(column("map.png", "Bob"), column("notes.txt", "you"));
}

#[test]
fn a_saved_file_carries_a_check_mark_and_the_others_do_not() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    mark_saved(&mut app, "map.png");

    let rows = files_box(&app);

    assert!(row_with(&rows, "map.png").contains('✓'));
    assert!(!row_with(&rows, "notes.txt").contains('✓'));
    assert!(!row_with(&rows, "report.pdf").contains('✓'));
}

#[test]
fn the_highlight_follows_down_and_is_styled_like_the_pickers() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    let area = files_area(screen());
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
fn a_saved_row_that_is_highlighted_keeps_the_highlight_colors() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    mark_saved(&mut app, "notes.txt"); // the first row

    let buffer = render_to_buffer(&app, 100, 20);

    let area = files_area(screen());
    let mark = &buffer[(area.x + 1 + 1, area.y + 2)];
    assert_eq!(mark.symbol(), "✓");
    assert_eq!((mark.fg, mark.bg), (Color::Black, Color::Cyan));
}

#[test]
fn typing_narrows_the_files_and_the_cursor_sits_after_the_filter() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    type_text(&mut app, "ALI");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 24))
        .expect("a test backend never fails");
    terminal
        .draw(|frame| render(frame, &app))
        .expect("drawing to a test backend never fails");

    let area = files_area(Rect::new(0, 0, 110, 24));
    let text = box_rows(terminal.backend().buffer(), area).join("\n");
    let cursor = terminal
        .get_cursor_position()
        .expect("a test backend never fails");

    assert!(text.contains("report.pdf"), "Alice's file:\n{text}");
    assert!(
        !text.contains("map.png") && !text.contains("notes.txt"),
        "got:\n{text}"
    );
    // Past the border, the 2-cell prompt and the 3 typed characters, on
    // the row under the top border -- not down in the input box.
    assert_eq!((cursor.x, cursor.y), (area.x + 1 + 2 + 3, area.y + 1));
}

#[test]
fn a_channel_with_no_files_says_nothing_has_been_shared() {
    let mut app = app_with_every_kind();
    open_files(&mut app);

    let rows = files_box(&app);

    assert!(
        rows[2].contains("no files have been shared in this channel yet"),
        "got:\n{}",
        rows.join("\n")
    );
}

#[test]
fn a_files_filter_that_matches_nothing_says_so() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    type_text(&mut app, "zzz");

    let rows = files_box(&app);

    assert!(rows[2].contains("no matches"), "got:\n{}", rows.join("\n"));
}

#[test]
fn the_bottom_border_offers_save_for_an_unsaved_file_and_open_for_a_saved_one() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    mark_saved(&mut app, "map.png");

    // `notes.txt` is highlighted first, and isn't saved.
    let unsaved = files_box(&app);
    let bottom = unsaved.last().unwrap();
    assert!(bottom.contains("Enter save"), "got: {bottom}");
    assert!(!bottom.contains("Ctrl+O"), "nothing to open yet: {bottom}");

    press(&mut app, KeyCode::Down); // `map.png`, saved
    let saved = files_box(&app);
    let bottom = saved.last().unwrap();
    assert!(bottom.contains("Ctrl+O open"), "got: {bottom}");
    assert!(
        !bottom.contains("Enter"),
        "saving it again is no offer: {bottom}"
    );
    assert!(bottom.contains("Esc close"), "got: {bottom}");
}

#[test]
fn enter_on_a_saved_file_says_so_in_the_bottom_border_until_the_next_key() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    mark_saved(&mut app, "notes.txt"); // the highlighted row

    press(&mut app, KeyCode::Enter);

    let rows = files_box(&app);
    let bottom = rows.last().unwrap();
    assert!(
        bottom.contains("already saved · Ctrl+O opens it"),
        "got: {bottom}"
    );
    assert!(
        !bottom.contains("Esc close"),
        "the notice takes the hints' place: {bottom}"
    );
    // Yellow and bold, as the picker's notices are.
    let area = files_area(Rect::new(0, 0, 110, 24));
    let column = bottom
        .find("already")
        .map(|at| bottom[..at].chars().count());
    let cell = &render_to_buffer(&app, 110, 24)[(
        area.x + column.expect("found above") as u16,
        area.bottom() - 1,
    )];
    assert_eq!(cell.fg, Color::Yellow);
    assert!(cell.modifier.contains(Modifier::BOLD));

    press(&mut app, KeyCode::Down);

    let rows = files_box(&app);
    let bottom = rows.last().unwrap();
    assert!(
        bottom.contains("Enter save"),
        "the hints are back: {bottom}"
    );
    assert!(!bottom.contains("already saved"), "got: {bottom}");
}

#[test]
fn ctrl_o_on_an_unsaved_file_says_so_in_the_bottom_border() {
    let mut app = app_with_shared_files();
    open_files(&mut app);

    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));

    let rows = files_box(&app);
    let bottom = rows.last().unwrap();
    assert!(
        bottom.contains("not available locally · Enter saves shared files"),
        "got: {bottom}"
    );
    assert!(!bottom.contains("Esc close"), "got: {bottom}");
}

/// The overlay open on `general`, where the newest file is `setup.exe`,
/// the next `notes.txt`, and those two are saved.
fn app_viewing_a_saved_program() -> AppState {
    let mut app = app_with_shared_files();
    app.load_history(
        "general",
        vec![shared_file(
            4,
            crate::message::now_unix_ms(),
            [7; 32],
            "setup.exe",
            10,
        )],
    );
    open_files(&mut app);
    let saved = |name: &str| {
        (
            iroh_blobs::Hash::new(name.as_bytes()),
            PathBuf::from(format!("/files/general/{name}")),
        )
    };
    app.apply_saved_files(
        "general",
        HashMap::from([saved("setup.exe"), saved("notes.txt")]),
    );
    app
}

#[test]
fn a_saved_program_is_offered_no_key_but_a_saved_document_still_is() {
    let mut app = app_viewing_a_saved_program();

    // `setup.exe` is the newest, so the highlighted row.
    let rows = files_box(&app);
    let bottom = rows.last().unwrap();
    assert!(
        bottom.contains("↑/↓ move") && bottom.contains("Esc close"),
        "got: {bottom}"
    );
    assert!(
        !bottom.contains("Enter") && !bottom.contains("Ctrl+O"),
        "neither key has anything to do for it: {bottom}"
    );

    press(&mut app, KeyCode::Down); // `notes.txt`, saved too

    let rows = files_box(&app);
    let bottom = rows.last().unwrap();
    assert!(bottom.contains("Ctrl+O open"), "got: {bottom}");
}

#[test]
fn both_keys_on_a_saved_program_say_it_will_not_be_opened_from_here() {
    let mut app = app_viewing_a_saved_program();
    let said = "won't open programs or scripts · Ctrl+R reveal · Ctrl+E edit";

    press(&mut app, KeyCode::Enter);
    let rows = files_box(&app);
    assert!(rows.last().unwrap().contains(said), "Enter: {rows:#?}");

    app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL));
    let rows = files_box(&app);
    assert!(rows.last().unwrap().contains(said), "Ctrl+O: {rows:#?}");
}

#[test]
fn with_nothing_to_act_on_the_bottom_border_only_offers_to_close() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    type_text(&mut app, "zzz");

    let rows = files_box(&app);

    let bottom = rows.last().unwrap();
    assert!(bottom.contains("Esc close"), "got: {bottom}");
    assert!(
        !bottom.contains("Enter") && !bottom.contains("Ctrl+O"),
        "{bottom}"
    );
}

#[test]
fn the_title_names_the_channel_by_its_heading_and_counts_the_files() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    assert!(files_box(&app)[0].contains("files in #general · 3"));

    type_text(&mut app, "report");
    assert!(
        files_box(&app)[0].contains("files in #general · 1 of 3"),
        "got: {}",
        files_box(&app)[0]
    );
}

#[test]
fn a_dm_and_a_dm_chat_are_titled_by_who_they_are_with() {
    let mut app = app_with_shared_files();
    app.load_history(
        "dm-07070707",
        vec![shared_file(10, 1, [7; 32], "dm.pdf", 5)],
    );
    app.load_history(
        "p2p-07070707",
        vec![shared_file(11, 1, [7; 32], "chat.pdf", 5)],
    );

    app.active = 1; // the /msg DM
    open_files(&mut app);
    let dm = files_box(&app);
    press(&mut app, KeyCode::Esc);
    app.active = 3; // the /dm chat
    open_files(&mut app);
    let direct = files_box(&app);

    assert!(dm[0].contains("files in @Alice"), "got: {}", dm[0]);
    row_with(&dm, "dm.pdf");
    assert!(direct[0].contains("files in ⚡Alice"), "got: {}", direct[0]);
    row_with(&direct, "chat.pdf");
}

#[test]
fn the_files_overlay_scrolls_to_keep_the_highlight_in_view() {
    let mut app = app_with_every_kind();
    let files = (0..40)
        .map(|i| shared_file(i + 1, i + 1, [7; 32], &format!("file-{i:02}.txt"), 1))
        .collect();
    app.load_history("general", files);
    open_files(&mut app);
    for _ in 0..30 {
        press(&mut app, KeyCode::Down);
    }

    let buffer = render_to_buffer(&app, 100, 20);

    // Newest first, so the thirty-first row is `file-09.txt`.
    let area = files_area(screen());
    let rows = box_rows(&buffer, area);
    let row = (area.y..area.bottom())
        .find(|&y| rows[(y - area.y) as usize].contains("file-09.txt"))
        .expect("the highlighted file is in view");
    assert!(row_is_highlighted(&buffer, area, row));
    assert!(!rows.join("\n").contains("file-39.txt"));
    assert!(
        (area.y..area.bottom()).any(|y| buffer[(area.right() - 1, y)].symbol() == "▐"),
        "a scroll thumb on the box's right border"
    );
}

#[test]
fn a_long_name_is_cut_short_of_the_columns_which_all_stay() {
    let mut app = app_with_every_kind();
    let long = "a-file-name-that-is-far-too-long-to-fit-in-the-overlay-next-to-its-size-sender-and-date.txt";
    app.load_history(
        "general",
        vec![shared_file(
            1,
            crate::message::now_unix_ms(),
            [9; 32],
            long,
            480,
        )],
    );
    open_files(&mut app);

    let rows = files_box(&app);

    let row = row_with(&rows, "a-file-name");
    assert!(row.contains('…'), "got: {row}");
    assert!(!row.contains("date.txt"), "got: {row}");
    assert!(row.contains("480 B") && row.contains("you"), "got: {row}");
}

#[test]
fn the_files_overlay_covers_what_is_underneath() {
    let mut app = app_with_shared_files();
    // Long enough to run right through the box's columns.
    app.load_history("general", vec![text_message(9, [1; 32], &"X".repeat(80))]);
    open_files(&mut app);

    let text = files_box(&app).join("\n");

    assert!(
        !text.contains('X'),
        "the message pane showed through:\n{text}"
    );
}

#[test]
fn the_files_overlay_survives_a_tiny_terminal() {
    let mut app = app_with_shared_files();
    open_files(&mut app);
    mark_saved(&mut app, "map.png");
    type_text(&mut app, "a");
    press(&mut app, KeyCode::Enter); // on saved `map.png`: a notice

    for width in 1..=30 {
        for height in 1..=8 {
            render_to_buffer(&app, width, height);
        }
    }
}

#[test]
fn the_files_box_is_centered_top_anchored_and_never_outgrows_the_screen() {
    let screen = Rect::new(0, 0, 120, 40);
    let area = files_area(screen);
    assert_eq!(area.width, FILES_WIDTH);
    assert_eq!(area.height, FILES_ROWS as u16 + 3);
    assert_eq!(area.x * 2 + area.width, screen.width, "centered");
    assert_eq!(area.y, 2, "anchored, so it never jumps while typing");

    for width in 1..=120 {
        for height in 1..=40 {
            let screen = Rect::new(0, 0, width, height);
            let area = files_area(screen);
            assert!(
                area.right() <= screen.right() && area.bottom() <= screen.bottom(),
                "{area:?} overflows {screen:?}"
            );
        }
    }
}

#[test]
fn a_long_heading_is_cut_short_of_the_counts_in_the_title() {
    let heading = format!("#{}", "channel-".repeat(20));

    let title = files_title(&heading, 2, 5, 60);

    assert!(title.chars().count() <= 60 - 2, "got: {title}");
    assert!(title.contains('…'), "got: {title}");
    assert!(title.ends_with(" · 2 of 5 "), "got: {title}");
}

fn hints_inner(area: Rect, app: &AppState) -> Rect {
    hints_areas(area, app).1
}

fn catalog_index(prefix: &str) -> usize {
    command_catalog()
        .iter()
        .position(|command| command.syntax.starts_with(prefix))
        .unwrap()
}

#[test]
fn hovering_a_hint_shows_its_description_then_whole_usage_in_a_popup() {
    let mut app = AppState::new([1; 32], vec!["general".into()], "general");
    let area = Rect::new(0, 0, 100, 40);
    let inner = hints_inner(area, &app);
    let index = catalog_index("/maxsize ");
    let command = &command_catalog()[index];
    let initial = render_to_buffer(&app, area.width, area.height);

    let target = command_hint_at(area, &app, inner.x, inner.y + index as u16);
    assert_eq!(target, Some(index));
    app.set_hint_hover(target);
    let buffer = render_to_buffer(&app, area.width, area.height);
    let rows = screen_rows(&buffer);

    let description_row = rows
        .iter()
        .position(|row| row.contains(command.description))
        .expect("the popup shows the description");
    assert!(
        rows[description_row + 1].contains(command.syntax),
        "the whole usage sits on the line under the description"
    );
    // The panel row itself stays clipped, not scrolled.
    assert_eq!(
        cells(&buffer, inner.y + index as u16, inner.x, inner.width),
        cells(&initial, inner.y + index as u16, inner.x, inner.width)
    );

    app.set_hint_hover(None);
    assert_eq!(render_to_buffer(&app, area.width, area.height), initial);
}

#[test]
fn a_hint_that_already_fits_still_gets_a_popup() {
    let mut app = AppState::new([1; 32], vec!["general".into()], "general");
    let area = Rect::new(0, 0, 100, 40);
    let inner = hints_inner(area, &app);
    let index = catalog_index("/invite");
    assert!(command_catalog()[index].syntax.len() < inner.width as usize);

    app.set_hint_hover(command_hint_at(area, &app, inner.x, inner.y + index as u16));
    let buffer = render_to_buffer(&app, area.width, area.height);

    assert!(
        screen_rows(&buffer)
            .iter()
            .any(|row| row.contains(command_catalog()[index].description))
    );
}

fn peers_inner(area: Rect, app: &AppState) -> Rect {
    peers_areas(area, app).1
}

#[test]
fn hovering_a_peer_lists_alias_nickname_status_and_version_in_a_popup() {
    use crate::message::{IdentityAnnounce, VersionAnnounce};
    use crate::net::NetEvent;

    let peer = [7; 32];
    let mut app = AppState::new([1; 32], vec!["general".into()], "general");
    app.active_mut().peers.push(peer);
    app.load_contacts(HashMap::from([(peer, "Alice".to_string())]));
    app.handle_net_event(NetEvent::Identity(
        "general".into(),
        IdentityAnnounce {
            sender: peer,
            nickname: "ally".into(),
        },
    ));
    app.handle_net_event(NetEvent::Version(
        "general".into(),
        VersionAnnounce {
            sender: peer,
            version: "1.2.3".into(),
            git_hash: "abc1234".into(),
        },
    ));
    let area = Rect::new(0, 0, 100, 40);
    let inner = peers_inner(area, &app);
    let initial = render_to_buffer(&app, area.width, area.height);

    app.set_peer_hover(peer_at(area, &app, inner.x, inner.y));
    let buffer = render_to_buffer(&app, area.width, area.height);
    let rows = screen_rows(&buffer);
    let alias = rows
        .iter()
        .position(|row| row.contains("Alias: Alice"))
        .expect("the popup shows the alias");
    assert!(
        rows[alias + 1].contains(&format!("ally ({})", hex_prefix(&peer))),
        "the short id follows the nickname in parentheses"
    );
    assert!(rows[alias + 2].contains("Online"));
    assert!(rows[alias + 3].contains("v1.2.3"));
    assert_eq!(
        rows.iter()
            .filter(|row| row.contains(&hex_prefix(&peer)))
            .count(),
        1,
        "the short id appears only beside the nickname"
    );

    // Alias and nickname are drawn in the peer's identity colour.
    for (row, name) in [(alias, "Alice"), (alias + 1, "ally")] {
        // A character (not byte) offset: the popup's border is multi-byte.
        let byte = rows[row].find(name).unwrap();
        let column = rows[row][..byte].chars().count() as u16;
        assert_eq!(
            buffer[(column, row as u16)].fg,
            user_color(&peer),
            "{name} should use the identity colour"
        );
    }

    app.set_peer_hover(None);
    assert_eq!(render_to_buffer(&app, area.width, area.height), initial);
}

#[test]
fn an_offline_peers_popup_omits_an_unset_alias_and_the_version() {
    use crate::message::VersionAnnounce;
    use crate::net::NetEvent;

    let peer = [7; 32];
    let mut app = AppState::new([1; 32], vec!["general".into()], "general");
    app.record_peer_seen("general", peer, 1_000);
    app.handle_net_event(NetEvent::Version(
        "general".into(),
        VersionAnnounce {
            sender: peer,
            // Older than this build, so no "consider updating" notice
            // lands in the transcript to confuse the check below.
            version: "0.0.1".into(),
            git_hash: "abc1234".into(),
        },
    ));
    // Tall enough that the peers panel fits two rows above the hints.
    let area = Rect::new(0, 0, 100, 50);
    let inner = peers_inner(area, &app);

    // Row 0 is our own (online) row; the offline peer follows it.
    app.set_peer_hover(peer_at(area, &app, inner.x, inner.y + 1));
    let rows = screen_rows(&render_to_buffer(&app, area.width, area.height));

    assert!(!rows.iter().any(|row| row.contains("Alias:")));
    // The panel row reads "○ 07070707"; only the popup puts the short
    // id straight after its border, in place of a nickname.
    let nickname = rows
        .iter()
        .position(|row| row.contains(&format!("│ {} ", hex_prefix(&peer))))
        .expect("the popup shows the short id in place of a nickname");
    assert!(
        !rows[nickname].contains('('),
        "no parentheses without a nickname"
    );
    assert!(!rows.iter().any(|row| row.contains(&hex_id(&peer))));
    assert!(
        rows[nickname + 1].contains(&format!("Offline: last seen {}", format_date_time(1_000)))
    );
    assert!(!rows.iter().any(|row| row.contains("v0.0.1")));
    assert!(!rows.iter().any(|row| row.contains("version unknown")));
}

#[test]
fn format_date_time_puts_the_local_date_after_format_times_time() {
    let ts = 1_760_000_000_000;
    let formatted = format_date_time(ts);
    let date = Local
        .timestamp_millis_opt(ts as i64)
        .unwrap()
        .format("%Y-%m-%d");
    assert_eq!(formatted, format!("{} {date}", format_time(ts)));
}

#[test]
fn peer_hit_testing_excludes_our_own_row_borders_and_overlays() {
    let peer = [7; 32];
    let mut app = AppState::new([1; 32], vec!["general".into()], "general");
    app.active_mut().peers.push(peer);
    let area = Rect::new(0, 0, 100, 40);
    let inner = peers_inner(area, &app);
    assert_eq!(peer_at(area, &app, inner.x, inner.y), Some(peer));
    assert_eq!(peer_at(area, &app, inner.x, inner.y + 1), None); // you
    assert_eq!(peer_at(area, &app, inner.x - 1, inner.y), None);
    assert_eq!(peer_at(area, &app, inner.x, inner.y - 1), None);
    open_switcher(&mut app);
    assert_eq!(peer_at(area, &app, inner.x, inner.y), None);
}

#[test]
fn hint_hit_testing_excludes_borders_and_overlays() {
    let mut app = AppState::new([1; 32], vec!["general".into()], "general");
    let area = Rect::new(0, 0, 100, 40);
    let inner = hints_inner(area, &app);
    assert_eq!(command_hint_at(area, &app, inner.x, inner.y + 3), Some(3));
    assert_eq!(command_hint_at(area, &app, inner.x, inner.y + 1), Some(1));
    assert_eq!(command_hint_at(area, &app, inner.x - 1, inner.y + 3), None);
    assert_eq!(
        command_hint_at(area, &app, inner.right(), inner.y + 3),
        None
    );
    assert_eq!(command_hint_at(area, &app, inner.x, inner.y - 1), None);
    assert_eq!(command_hint_at(area, &app, inner.x, inner.bottom()), None);
    open_switcher(&mut app);
    assert_eq!(command_hint_at(area, &app, inner.x, inner.y + 3), None);
}

#[test]
fn hint_hit_testing_and_popups_track_small_screens_and_multiline_composition() {
    let mut app = AppState::new([1; 32], vec!["general".into()], "general");
    for input in ["", "one\ntwo\nthree"] {
        app.active_mut().input = input.into();
        for width in [1, 20, 40, 80, 120] {
            for height in [1, 5, 12, 24, 40] {
                let area = Rect::new(0, 0, width, height);
                let inner = hints_inner(area, &app);
                let buffer = render_to_buffer(&app, width, height);
                for row in 0..height {
                    if let Some(index) = command_hint_at(area, &app, inner.x, row) {
                        assert_eq!(row, inner.y + index as u16);
                        assert!(
                            cells(&buffer, row, inner.x, inner.width)
                                .starts_with(&command_catalog()[index].syntax[..1])
                        );
                        // Rendering the popup must never panic or
                        // spill outside the terminal.
                        app.set_hint_hover(Some(index));
                        render_to_buffer(&app, width, height);
                        app.set_hint_hover(None);
                    }
                }
            }
        }
    }
}

#[test]
fn the_files_command_is_among_the_hints() {
    assert!(
        command_catalog()
            .iter()
            .any(|command| command.syntax == "/files")
    );
}

#[test]
fn the_maxsize_command_is_among_the_hints() {
    assert!(
        command_catalog()
            .iter()
            .any(|command| command.syntax.starts_with("/maxsize"))
    );
}

#[test]
fn the_autosave_command_is_among_the_hints() {
    assert!(
        command_catalog()
            .iter()
            .any(|command| command.syntax.starts_with("/autosave"))
    );
}

#[test]
fn the_outbox_command_is_among_the_hints() {
    assert!(
        command_catalog()
            .iter()
            .any(|command| command.syntax.starts_with("/outbox"))
    );
}

#[test]
fn the_sync_policy_command_is_among_the_hints() {
    assert!(
        command_catalog()
            .iter()
            .any(|command| command.syntax.starts_with("/sync"))
    );
}

#[test]
fn the_sync_trash_command_is_among_the_hints() {
    assert!(
        command_catalog()
            .iter()
            .any(|command| command.syntax.starts_with("/trash"))
    );
}

#[test]
fn the_conflicts_command_is_among_the_hints() {
    assert!(
        command_catalog()
            .iter()
            .any(|command| command.syntax.starts_with("/conflicts"))
    );
}

#[test]
fn the_mouse_command_is_among_the_hints() {
    assert!(
        command_catalog()
            .iter()
            .any(|command| command.syntax == "/mouse")
    );
}

#[test]
fn peer_hover_works_with_hints_hidden_and_stops_without_the_mouse() {
    let peer = [7; 32];
    let mut app = AppState::new([1; 32], vec!["general".into()], "general");
    app.active_mut().peers.push(peer);
    app.show_hints = false;
    let area = Rect::new(0, 0, 100, 40);
    let inner = peers_inner(area, &app);
    let [_, body, _, _] = frame_areas(area, &app);
    let [_, _, sidebar] = body_areas(body);
    assert_eq!(
        inner.bottom() + 1,
        sidebar.bottom(),
        "the peers panel fills the sidebar"
    );

    assert_eq!(peer_at(area, &app, inner.x, inner.y), Some(peer));
    app.set_peer_hover(Some(peer));
    let buffer = render_to_buffer(&app, area.width, area.height);
    assert!(
        screen_rows(&buffer)
            .iter()
            .any(|row| row.contains("Online"))
    );

    app.mouse_enabled = false;
    assert_eq!(peer_at(area, &app, inner.x, inner.y), None);
}

#[test]
fn a_share_is_dated_the_way_a_file_is_in_the_picker() {
    let now = Local.with_ymd_and_hms(2026, 10, 12, 15, 0, 0).unwrap();
    let at = |year, month, day, hour, minute| {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .unwrap()
            .timestamp_millis() as u64
    };

    let today = sent_label(at(2026, 10, 12, 14, 3), now);
    let last_year = sent_label(at(2025, 3, 3, 11, 20), now);

    assert_eq!(today, "12 Oct 14:03");
    assert_eq!(last_year, "03 Mar  2025");
    for label in [today, last_year] {
        assert_eq!(label.chars().count(), PICKER_TIME_WIDTH, "{label}");
    }
}

#[test]
fn a_timestamp_that_cannot_be_dated_is_dashes_as_wide_as_a_date() {
    let now = Local.with_ymd_and_hms(2026, 10, 12, 15, 0, 0).unwrap();

    // `i64::MIN` milliseconds, as a peer's message could claim.
    let label = sent_label(1 << 63, now);

    assert_eq!(label.trim_end(), "--");
    assert_eq!(label.chars().count(), PICKER_TIME_WIDTH);
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
