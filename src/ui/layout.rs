use super::*;

pub(super) fn frame_areas(area: Rect, app: &AppState) -> [Rect; 4] {
    Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(staged_strip_height(app.active().staged.len())),
        Constraint::Length(input_box_height(&app.active().input)),
    ])
    .areas(area)
}

pub(super) fn body_areas(area: Rect) -> [Rect; 3] {
    Layout::horizontal([
        Constraint::Length(NAV_WIDTH),
        Constraint::Min(20),
        Constraint::Length(22),
    ])
    .areas(area)
}

pub(super) fn sidebar_areas(area: Rect) -> [Rect; 2] {
    Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(command_catalog().len() as u16 + 2),
    ])
    .areas(area)
}

/// The command hints panel and its inner (borderless) area, laid out
/// exactly as `render_sidebar` draws them.
pub(super) fn hints_areas(area: Rect, app: &AppState) -> (Rect, Rect) {
    let [_, body, _, _] = frame_areas(area, app);
    let [_, _, sidebar] = body_areas(body);
    let [_, hints] = sidebar_areas(sidebar);
    (hints, Block::bordered().inner(hints))
}

/// The peers panel and its inner (borderless) area, laid out exactly as
/// `render_sidebar` draws them: the whole sidebar while the hints panel
/// is hidden, otherwise the room above it.
pub(super) fn peers_areas(area: Rect, app: &AppState) -> (Rect, Rect) {
    let [_, body, _, _] = frame_areas(area, app);
    let [_, _, sidebar] = body_areas(body);
    let peers = if app.show_hints {
        let [peers, _] = sidebar_areas(sidebar);
        peers
    } else {
        sidebar
    };
    (peers, Block::bordered().inner(peers))
}

/// Uses exactly the renderer's layout, excluding borders, our own row,
/// and modal views. Returns the id of the peers-panel peer under
/// `(column, row)`.
pub fn peer_at(area: Rect, app: &AppState, column: u16, row: u16) -> Option<[u8; 32]> {
    if !app.can_hover() {
        return None;
    }
    let (_, inner) = peers_areas(area, app);
    if !inner.contains((column, row).into()) {
        return None;
    }
    peers_panel_rows(app)
        .get((row - inner.y) as usize)
        .map(|peer| peer.id)
        .filter(|id| *id != app.self_id)
}

/// The peers panel's rows, top to bottom, as `render_peers` draws them:
/// every online peer, then our own row (we're always online), then every
/// remembered offline peer -- `peer_presence` already lists online peers
/// before offline ones, most recently seen first.
pub(super) fn peers_panel_rows(app: &AppState) -> Vec<PeerPresence> {
    let mut rows = app.peer_presence();
    let first_offline = rows
        .iter()
        .position(|peer| peer.last_seen_unix_ms.is_some())
        .unwrap_or(rows.len());
    rows.insert(
        first_offline,
        PeerPresence {
            id: app.self_id,
            last_seen_unix_ms: None,
        },
    );
    rows
}

/// Uses exactly the renderer's layout, excluding borders, hidden rows,
/// and modal views. Returns the catalog index of the command row under
/// `(column, row)`.
pub fn command_hint_at(area: Rect, app: &AppState, column: u16, row: u16) -> Option<usize> {
    if !app.can_hover_hints() {
        return None;
    }
    let (_, inner) = hints_areas(area, app);
    if !inner.contains((column, row).into()) {
        return None;
    }
    let index = (row - inner.y) as usize;
    command_catalog().get(index).map(|_| index)
}

/// Height (in terminal rows, including the 2 border rows) the input box
/// needs for `input`'s current line count, capped at
/// `MAX_INPUT_VISIBLE_LINES` -- computed before laying out the rest of
/// the frame (`render`) so the message pane shrinks to make room while
/// composing a multi-line message, and grows back once it's sent.
pub(super) fn input_box_height(input: &str) -> u16 {
    let lines = input.split('\n').count().max(1);
    lines.min(MAX_INPUT_VISIBLE_LINES) as u16 + 2
}

/// Height (in terminal rows, including the 2 border rows) of the strip of
/// staged attachments above the input box -- zero when nothing is staged,
/// so the strip takes no room until there's something to show. Computed
/// before laying out the rest of the frame (`render`), like
/// `input_box_height`, so the message pane gives up exactly that much.
pub(super) fn staged_strip_height(staged: usize) -> u16 {
    if staged == 0 {
        0
    } else {
        staged.min(MAX_STAGED_ROWS) as u16 + 2
    }
}
