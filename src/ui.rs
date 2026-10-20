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

use crate::app::{
    AppState, Channel, CommandDefinition, CommandPickerAction, FileOffer, FilesEntry, FilesNotice,
    PeerDetails, PeerPresence, TranscriptLine, command_catalog, hex_id, hex_prefix,
};
use crate::files::{StagedAttachment, human_size};
use crate::hyperlink;
use crate::markdown;
use crate::message::ChatMessage;
use crate::picker::{Entry, FilePicker, Kind};
use crate::search::SearchResults;
use crate::thread::ThreadView;

mod files_overlay;
use files_overlay::*;

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

/// Width of the jump-to overlay (`render_switcher`), borders included.
const SWITCHER_WIDTH: u16 = 44;
/// Most matches the jump-to overlay shows at once before it scrolls.
const SWITCHER_MAX_ROWS: usize = 10;
/// Width of the command picker overlay, borders included.
const COMMAND_PICKER_WIDTH: u16 = 78;
/// Most command-picker rows shown before it scrolls.
const COMMAND_PICKER_ROWS: usize = 18;
/// Width reserved for shortcut labels in command-picker rows.
const COMMAND_PICKER_SHORTCUT_WIDTH: usize = 8;
/// Width reserved for the icon and label that explain what Enter will do.
const COMMAND_PICKER_ACTION_WIDTH: usize = 7;

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

/// Width of the files overlay (`render_files`), borders included -- wider
/// than the picker's, since its rows carry a sender as well. Its rows are
/// the picker's besides: the same gutter, size and date columns.
const FILES_WIDTH: u16 = 92;
/// Rows of files the overlay lists at once before it scrolls.
const FILES_ROWS: usize = 14;
/// Width of the `/who` peer-details overlay, borders included.
const PEERS_OVERLAY_WIDTH: u16 = 104;

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

mod layout;
use layout::*;
mod navigation;
use navigation::*;
mod overlays;
use overlays::*;
mod picker;
use picker::*;
mod messages;
use messages::*;
mod sidebar;
use sidebar::*;

pub use layout::{command_hint_at, peer_at};

pub fn render(frame: &mut Frame, app: &AppState) {
    let [header, body, staged, input] = frame_areas(frame.area(), app);

    render_header(frame, header, app);
    render_body(frame, body, app);
    render_staged(frame, staged, app);
    render_input(frame, input, app);
    render_hint_popup(frame, app);
    render_peer_popup(frame, app);
    // Last, so the overlay sits over everything else -- and so its cursor
    // wins over the input box's. At most one is ever open: each owns every
    // key while it is.
    render_switcher(frame, app);
    render_command_picker(frame, app);
    render_picker(frame, app);
    render_files(frame, app);
    render_peers_overlay(frame, app);
}

#[cfg(test)]
mod tests;
