//! State transitions and derived data for the `/files` overlay.

use super::*;

impl AppState {
    /// (`apply_saved_files`).
    pub(super) fn run_files(&mut self) -> Option<InputAction> {
        let channel = self.active().name.clone();
        self.viewing_files = Some(FilesOverlay {
            channel: channel.clone(),
            query: String::new(),
            selected: 0,
            saved: HashMap::new(),
            notice: None,
        });
        Some(InputAction::CheckSavedFiles(channel))
    }

    /// Handles a keypress while the files overlay is open: typing and
    /// `Backspace` edit the query, `Up`/`Down` move the highlight, `Enter`
    /// saves the highlighted file (`save_highlighted_file`), `Ctrl+O` opens
    /// it if it is already saved (`open_highlighted_file`), `Esc` closes
    /// the overlay. `Ctrl+C` still force-quits, since `Esc` no longer does
    /// while it's open. Every other key is ignored -- like the switcher this
    /// is a dedicated mode, so a stray keystroke can't leak into the input
    /// box. Whatever key it is, it first drops the notice about the one
    /// before (`FilesNotice`), which only the key that raises one puts back.
    pub(super) fn handle_files_key(&mut self, key: KeyEvent) -> Option<InputAction> {
        if let Some(overlay) = &mut self.viewing_files {
            overlay.notice = None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Char('o') if ctrl => return self.open_highlighted_file(),
            KeyCode::Esc => self.viewing_files = None,
            KeyCode::Enter => return self.save_highlighted_file(),
            KeyCode::Up => self.move_files_highlight(-1),
            KeyCode::Down => self.move_files_highlight(1),
            KeyCode::Backspace => self.edit_files_query(|query| {
                query.pop();
            }),
            KeyCode::Char(c) if !ctrl => self.edit_files_query(|query| query.push(c)),
            _ => {}
        }
        None
    }

    /// Applies `edit` to the open overlay's query and puts the highlight back
    /// on the first match. The one place the query changes, so a typed
    /// character, a `Backspace` and a paste can't disagree about where the
    /// highlight lands. A paste doesn't pass through `handle_files_key`, so
    /// this drops the notice too. A no-op if the overlay isn't open.
    pub(super) fn edit_files_query(&mut self, edit: impl FnOnce(&mut String)) {
        if let Some(overlay) = &mut self.viewing_files {
            edit(&mut overlay.query);
            overlay.selected = 0;
            overlay.notice = None;
        }
    }

    /// Every file shared in the overlay's channel, newest first, one per
    /// share: what its loaded transcript carries -- history loaded at
    /// startup, backfilled and live alike, the same window `/save` resolves
    /// against (`known_attachments`). Empty while the overlay is closed.
    pub fn channel_files(&self) -> Vec<SharedFile<'_>> {
        let Some(overlay) = &self.viewing_files else {
            return Vec::new();
        };
        let Some(channel) = self.channels.iter().find(|c| c.name == overlay.channel) else {
            return Vec::new();
        };
        channel
            .messages
            .iter()
            .rev()
            .filter_map(|line| match line {
                TranscriptLine::Chat(message) => {
                    message.attachment.as_ref().map(|file| SharedFile {
                        file,
                        sender: message.sender,
                        ts_unix_ms: message.ts_unix_ms,
                    })
                }
                TranscriptLine::System(_) => None,
            })
            .collect()
    }

    /// The files the open overlay lists, newest first: those of its channel
    /// whose name -- or whose sender as shown (`display_name`: what's on
    /// screen, never the raw id) -- contains the typed text, ignoring case.
    /// An empty query lists every file; nothing is listed while the overlay
    /// is closed.
    pub fn files_matches(&self) -> Vec<SharedFile<'_>> {
        let Some(overlay) = &self.viewing_files else {
            return Vec::new();
        };
        let query = overlay.query.to_lowercase();
        self.channel_files()
            .into_iter()
            .filter(|shared| {
                shared.file.filename.to_lowercase().contains(&query)
                    || self
                        .display_name(&shared.sender)
                        .to_lowercase()
                        .contains(&query)
            })
            .collect()
    }

    /// The position among `files_matches` that `Enter` would act on: the
    /// highlighted match, or `None` while nothing matches or the overlay is
    /// closed. The stored position is clamped here, and only here -- the
    /// matches can change under an open overlay (a file arrives, a peer gets
    /// renamed), and a position left over from before must never point past
    /// the end.
    pub fn files_highlight(&self) -> Option<usize> {
        let overlay = self.viewing_files.as_ref()?;
        let last = self.files_matches().len().checked_sub(1)?;
        Some(overlay.selected.min(last))
    }

    /// Moves the highlight by `delta` among the current matches, clamping at
    /// both ends rather than wrapping, like `move_switcher`. A no-op while
    /// nothing matches.
    pub(super) fn move_files_highlight(&mut self, delta: isize) {
        let Some(last) = self.files_matches().len().checked_sub(1) else {
            return;
        };
        if let Some(overlay) = &mut self.viewing_files {
            let current = overlay.selected.min(last) as isize;
            overlay.selected = (current + delta).clamp(0, last as isize) as usize;
        }
    }

    /// The highlighted match -- what `Enter` and `Ctrl+O` act on.
    pub(super) fn highlighted_file(&self) -> Option<SharedFile<'_>> {
        let position = self.files_highlight()?;
        self.files_matches().into_iter().nth(position)
    }

    /// Where the overlay's channel's saved copy of the file `hash` is, if it
    /// has one that was on disk when last checked -- what the overlay marks
    /// as saved, and what `Ctrl+O` opens.
    pub fn saved_copy(&self, hash: Hash) -> Option<&Path> {
        self.viewing_files
            .as_ref()?
            .saved
            .get(&hash)
            .map(PathBuf::as_path)
    }

    /// What `Enter` and `Ctrl+O` can do for `shared`, the one answer the
    /// bottom border and both keys go by: save it if it isn't saved, open its
    /// saved copy if that is something to open, and otherwise leave it be --
    /// a copy that runs something when opened (`runs_when_opened`) is never
    /// handed to the OS from here, so it is saved and that is all. Judged by
    /// the saved copy's own name, not the one it was announced under: that is
    /// the one the OS will go by.
    pub fn file_offer(&self, shared: &SharedFile<'_>) -> FileOffer<'_> {
        match self.saved_copy(shared.file.hash) {
            None => FileOffer::Save,
            Some(path) if runs_when_opened(path) => FileOffer::SavedOnly,
            Some(path) => FileOffer::Open(path),
        }
    }

    /// `Enter` in the files overlay: saves the highlighted file exactly as
    /// `/save` would -- the same providers, folder and notices
    /// (`save_request`) -- and closes the overlay, so those notices, and a
    /// failure's, show in the transcript instead of behind the overlay. Does
    /// nothing, leaving the overlay open, while nothing is highlighted -- and
    /// when the file is already saved (`file_offer`): saving it again would
    /// only leave a second copy (`name (1).ext`) beside the first. The
    /// overlay says so (`FilesNotice`), pointing at `Ctrl+O` if that opens
    /// it, rather than leave `Enter` looking dead.
    pub(super) fn save_highlighted_file(&mut self) -> Option<InputAction> {
        let shared = self.highlighted_file()?;
        match self.file_offer(&shared) {
            FileOffer::Save => {}
            FileOffer::Open(_) => {
                self.notify_files(FilesNotice::AlreadySaved);
                return None;
            }
            FileOffer::SavedOnly => {
                self.notify_files(FilesNotice::SavedNotOpened);
                return None;
            }
        }
        let (hash, filename, sender) = (
            shared.file.hash,
            shared.file.filename.clone(),
            shared.sender,
        );
        let channel = self.viewing_files.take()?.channel;
        Some(self.save_request(channel, hash, filename, sender))
    }

    /// `Ctrl+O` in the files overlay: opens the highlighted file's saved copy
    /// with the OS's default application, and closes the overlay. Does
    /// nothing, leaving it open, while nothing is highlighted -- and when
    /// there is nothing to open (`file_offer`): the file isn't saved yet and
    /// `Enter` saves it, or its copy runs something when opened
    /// (`files::runs_when_opened`) and is never handed to the OS from here.
    /// The overlay says which (`FilesNotice`).
    pub(super) fn open_highlighted_file(&mut self) -> Option<InputAction> {
        let shared = self.highlighted_file()?;
        let path = match self.file_offer(&shared) {
            FileOffer::Open(path) => path.to_path_buf(),
            FileOffer::Save => {
                self.notify_files(FilesNotice::NotSaved);
                return None;
            }
            FileOffer::SavedOnly => {
                self.notify_files(FilesNotice::SavedNotOpened);
                return None;
            }
        };
        let filename = shared.file.filename.clone();
        self.viewing_files = None;
        self.push_system(format!("opening {filename}..."));
        Some(InputAction::OpenFile(path))
    }

    /// Has the open overlay say `notice` in its bottom border, until the next
    /// key. A no-op if the overlay isn't open.
    pub(super) fn notify_files(&mut self, notice: FilesNotice) {
        if let Some(overlay) = &mut self.viewing_files {
            overlay.notice = Some(notice);
        }
    }

    /// Hands the files overlay what the disk says about `channel`'s saved
    /// files (`saved_files::SavedFiles::present_in`) -- the answer to
    /// `InputAction::CheckSavedFiles`. A no-op if the overlay has since
    /// closed, or is open on another channel.
    pub fn apply_saved_files(&mut self, channel: &str, saved: HashMap<Hash, PathBuf>) {
        if let Some(overlay) = &mut self.viewing_files
            && overlay.channel == channel
        {
            overlay.saved = saved;
        }
    }
}
