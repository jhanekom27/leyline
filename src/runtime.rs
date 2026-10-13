//! Runtime side-effect orchestration used by the terminal event loop.

use super::*;

/// `run` below, so the two can't drift apart.
pub(super) fn announce_history(net: &Net, backfill: &BackfillStore, channel: &str) {
    if let Some(root) = backfill.current_root(channel) {
        net.announce(channel, root);
    }
}

/// Sends a locally-authored `message` to `channel` via whichever transport
/// it actually belongs to: a point-to-point connection
/// (`net::Net::send_direct`) if `channel` is one (`app::AppState::
/// direct_peer_for`), else the ordinary gossip broadcast (`net::Net::
/// send`). Centralizing this one decision here, rather than duplicating it
/// at every call site that broadcasts a locally-authored message, is what
/// lets `/send`, `/paste`, and a plain typed message all work unmodified
/// over a point-to-point conversation -- none of them depend on gossip
/// specifically, only on reaching the right peer.
pub(super) fn route_send(net: &Net, app: &AppState, channel: &str, message: ChatMessage) {
    match app.direct_peer_for(channel) {
        Some(peer) => net.send_direct(peer, message),
        None => net.send(channel, message),
    }
}

/// Starts the download behind `request`: claims a free name in its folder
/// first (`files::resolve_destination`, which makes the folder too), so a
/// folder that can't be written to is reported now rather than after a
/// download, then hands the fetch to `net`. It lands as `NetEvent::FileSaved`
/// or `FileSaveFailed`. `/save`, `Enter` in the files overlay and auto-save
/// all come through here, so a file is named, made safe and kept apart from
/// a namesake the same way whichever of them asked.
pub(super) fn start_save(net: &Net, app: &mut AppState, request: SaveRequest) {
    let SaveRequest {
        channel,
        hash,
        filename,
        providers,
        dir,
        mode,
    } = request;
    match resolve_destination(&dir, &filename) {
        Ok(destination) => net.save_file(channel, hash, filename, providers, destination, mode),
        Err(err) => app.save_failed(&channel, hash, &filename, &err.to_string(), mode),
    }
}

/// Stages real files on `channel` for review: the ones a `Ctrl+V`/`/paste`
/// found on the clipboard (`NetEvent::ClipboardFiles`, e.g. a
/// Finder/Explorer copy) -- on the channel the paste was for, which may no
/// longer be the active one by the time the clipboard read comes back --
/// and the ones a drag-and-drop pasted as paths (`handle_paste`). Each path
/// is checked exactly like a `/send`ed one (`files::stage_file`, against the
/// size limit `/maxsize` set); one that is refused (unreadable, too large)
/// is reported as a system notice without affecting the rest, so a
/// multi-file copy or drop stages whatever it can. Nothing is shared yet:
/// that waits for `Enter` (`app::AppState::send_staged`).
pub(super) fn stage_files(app: &mut AppState, channel: &str, paths: &[PathBuf]) {
    for path in paths {
        match stage_file(path, app.size_limit()) {
            Ok(item) => app.stage_attachment(channel, item),
            Err(message) => app.push_system(message),
        }
    }
}

/// Stages a clipboard image (`NetEvent::ClipboardImage`, already
/// re-encoded as PNG) the same way `stage_files` stages files; the one
/// thing to check -- the size limit -- is reported immediately, like a
/// refused `/send`.
pub(super) fn stage_clipboard_image(
    app: &mut AppState,
    channel: &str,
    filename: String,
    bytes: Vec<u8>,
) {
    match stage_image(filename, bytes, app.size_limit()) {
        Ok(item) => app.stage_attachment(channel, item),
        Err(message) => app.push_system(message),
    }
}

/// Handles a bracketed paste (`Event::Paste`). A terminal turns a
/// drag-and-drop into a paste of the dropped files' paths (or, in Warp's
/// case, types them out -- `term_events::TermEvents` turns that into the
/// same `Event::Paste`), so a paste that is nothing but existing files
/// (`files::dropped_files`) is a drop: the files are staged on the active
/// channel, like a `Ctrl+V` of a Finder copy, instead of landing in the
/// message box as a wall of path text.
/// Anything else is text for the message box, as it always was --
/// including a paste while the switcher, a pick or a pairing confirmation
/// has taken over input (`AppState::is_composing`), where staging files
/// would put them somewhere the user is not looking.
pub(super) fn handle_paste(app: &mut AppState, text: &str) {
    if app.is_composing()
        && let Some(paths) = dropped_files(text)
    {
        let channel = app.active().name.clone();
        stage_files(app, &channel, &paths);
    } else {
        app.paste_text(text);
    }
}

/// Rings the terminal bell (ASCII BEL) to signal a newly-arrived message --
/// see `AppState::bell_enabled`, toggled via `/bell` and persisted through
/// settings.rs. A plain control byte, so it's safe to write straight to
/// stdout even with `ratatui::init()` owning the alternate screen: it
/// doesn't move the cursor or draw anything, and it's entirely up to the
/// terminal emulator how (or whether) to actually alert -- the same as a
/// shell's `printf '\a'`. Best-effort: a failed write here isn't worth
/// surfacing, unlike a storage/backfill error.
pub(super) fn ring_bell() {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(b"\x07").and_then(|()| stdout.flush());
}
