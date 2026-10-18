//! State transitions and derived data for the `/files` overlay.

use super::*;

impl FilesEntry {
    pub fn filename(&self) -> &str {
        match self {
            Self::Shared(shared) => &shared.file.filename,
            Self::Local(local) => &local.relative,
        }
    }

    pub fn timestamp(&self) -> u64 {
        match self {
            Self::Shared(shared) => shared.ts_unix_ms,
            Self::Local(local) => local.modified_unix_ms,
        }
    }

    pub fn shared(&self) -> Option<&SharedFile> {
        match self {
            Self::Shared(shared) => Some(shared),
            Self::Local(_) => None,
        }
    }
}

impl AppState {
    /// Opens a channel-local file browser. The runtime answers with both its
    /// recorded saved attachments and a background scan of the channel folder.
    pub(super) fn run_files(&mut self) -> Option<InputAction> {
        let channel = self.active().name.clone();
        let request = next_request(&mut self.files_requests);
        let dir = self.folder_dir(&channel);
        self.viewing_files = Some(FilesOverlay {
            channel: channel.clone(),
            request,
            query: String::new(),
            selected: 0,
            saved: HashMap::new(),
            local: Vec::new(),
            notice: None,
        });
        Some(InputAction::ReadFiles {
            request,
            channel,
            dir,
        })
    }

    pub(super) fn handle_files_key(&mut self, key: KeyEvent) -> Option<InputAction> {
        if let Some(overlay) = &mut self.viewing_files {
            overlay.notice = None;
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Char('o') if ctrl => return self.open_highlighted_file(),
            KeyCode::Char('r') if ctrl => return self.reveal_highlighted_file(),
            KeyCode::Char('e') if ctrl => return self.edit_highlighted_file(),
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

    pub(super) fn edit_files_query(&mut self, edit: impl FnOnce(&mut String)) {
        if let Some(overlay) = &mut self.viewing_files {
            edit(&mut overlay.query);
            overlay.selected = 0;
            overlay.notice = None;
        }
    }

    /// Attachments seen in the overlay's channel, newest first. This stays
    /// separate for `/save`'s existing attachment semantics and focused tests.
    pub fn channel_files(&self) -> Vec<SharedFile> {
        let Some(overlay) = &self.viewing_files else {
            return Vec::new();
        };
        self.shared_files
            .get(&overlay.channel)
            .cloned()
            .unwrap_or_default()
    }

    /// Shared attachments and locally available channel-folder files, with a
    /// saved attachment and its exact scanned path collapsed into one row.
    pub fn overlay_files(&self) -> Vec<FilesEntry> {
        let Some(overlay) = &self.viewing_files else {
            return Vec::new();
        };
        let mut entries: Vec<_> = self
            .channel_files()
            .into_iter()
            .map(FilesEntry::Shared)
            .collect();
        entries.extend(
            overlay
                .local
                .iter()
                .filter(|local| !overlay.saved.values().any(|saved| saved == &local.path))
                .cloned()
                .map(FilesEntry::Local),
        );
        entries.sort_by_key(|entry| std::cmp::Reverse(entry.timestamp()));
        entries
    }

    /// The overlay entries whose display filename, relative local path, or
    /// sender name contains the query, ignoring case.
    pub fn files_matches(&self) -> Vec<FilesEntry> {
        let Some(overlay) = &self.viewing_files else {
            return Vec::new();
        };
        let query = overlay.query.to_lowercase();
        self.overlay_files()
            .into_iter()
            .filter(|entry| {
                entry.filename().to_lowercase().contains(&query)
                    || entry.shared().is_some_and(|shared| {
                        self.display_name(&shared.sender)
                            .to_lowercase()
                            .contains(&query)
                    })
            })
            .collect()
    }

    pub fn files_highlight(&self) -> Option<usize> {
        let overlay = self.viewing_files.as_ref()?;
        let last = self.files_matches().len().checked_sub(1)?;
        Some(overlay.selected.min(last))
    }

    pub(super) fn move_files_highlight(&mut self, delta: isize) {
        let Some(last) = self.files_matches().len().checked_sub(1) else {
            return;
        };
        if let Some(overlay) = &mut self.viewing_files {
            let current = overlay.selected.min(last) as isize;
            overlay.selected = (current + delta).clamp(0, last as isize) as usize;
        }
    }

    pub(super) fn highlighted_file(&self) -> Option<FilesEntry> {
        let position = self.files_highlight()?;
        self.files_matches().into_iter().nth(position)
    }

    pub fn saved_copy(&self, hash: Hash) -> Option<&Path> {
        self.viewing_files
            .as_ref()?
            .saved
            .get(&hash)
            .map(PathBuf::as_path)
    }

    /// The local path for one row: a saved attachment's recorded copy or the
    /// scanned path of a sync-local file.
    pub fn local_copy(&self, entry: &FilesEntry) -> Option<PathBuf> {
        match entry {
            FilesEntry::Shared(shared) => self.saved_copy(shared.file.hash).map(Path::to_path_buf),
            FilesEntry::Local(local) => Some(local.path.clone()),
        }
    }

    pub fn file_offer(&self, entry: &FilesEntry) -> FileOffer {
        match self.local_copy(entry) {
            None => FileOffer::Save,
            Some(path) if runs_when_opened(&path) => FileOffer::SavedOnly,
            Some(path) => FileOffer::Open(path),
        }
    }

    pub(super) fn save_highlighted_file(&mut self) -> Option<InputAction> {
        let entry = self.highlighted_file()?;
        let FilesEntry::Shared(shared) = entry else {
            self.notify_files(FilesNotice::LocalFile);
            return None;
        };
        match self.file_offer(&FilesEntry::Shared(shared.clone())) {
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
        let channel = self.viewing_files.take()?.channel;
        Some(self.save_request(
            channel,
            shared.file.hash,
            shared.file.filename,
            shared.sender,
        ))
    }

    pub(super) fn open_highlighted_file(&mut self) -> Option<InputAction> {
        let entry = self.highlighted_file()?;
        let path = match self.file_offer(&entry) {
            FileOffer::Open(path) => path,
            FileOffer::Save => {
                self.notify_files(FilesNotice::NotLocal);
                return None;
            }
            FileOffer::SavedOnly => {
                self.notify_files(FilesNotice::SavedNotOpened);
                return None;
            }
        };
        let filename = entry.filename().to_string();
        self.viewing_files = None;
        self.push_system(format!("opening {filename}..."));
        Some(InputAction::OpenFile(path))
    }

    pub(super) fn reveal_highlighted_file(&mut self) -> Option<InputAction> {
        let entry = self.highlighted_file()?;
        let Some(path) = self.local_copy(&entry) else {
            self.notify_files(FilesNotice::NotLocal);
            return None;
        };
        let filename = entry.filename().to_string();
        self.viewing_files = None;
        self.push_system(format!("revealing {filename}..."));
        Some(InputAction::RevealFile(path))
    }

    pub(super) fn edit_highlighted_file(&mut self) -> Option<InputAction> {
        let entry = self.highlighted_file()?;
        let Some(path) = self.local_copy(&entry) else {
            self.notify_files(FilesNotice::NotLocal);
            return None;
        };
        let filename = entry.filename().to_string();
        self.viewing_files = None;
        self.push_system(format!("editing {filename}..."));
        Some(InputAction::EditFile(path))
    }

    pub(super) fn notify_files(&mut self, notice: FilesNotice) {
        if let Some(overlay) = &mut self.viewing_files {
            overlay.notice = Some(notice);
        }
    }

    pub fn apply_saved_files(&mut self, channel: &str, saved: HashMap<Hash, PathBuf>) {
        if let Some(overlay) = &mut self.viewing_files
            && overlay.channel == channel
        {
            overlay.saved = saved;
        }
    }

    /// Accepts only the listing requested by the overlay still on screen.
    pub fn apply_files_listing(&mut self, listing: FilesListing) {
        let error = {
            let Some(overlay) = &mut self.viewing_files else {
                return;
            };
            if overlay.request != listing.request || overlay.channel != listing.channel {
                return;
            }
            match listing.files {
                Ok(files) => {
                    overlay.local = files;
                    None
                }
                Err(error) => {
                    overlay.notice = Some(FilesNotice::NotLocal);
                    Some(error)
                }
            }
        };
        if let Some(error) = error {
            self.push_system_to(
                &listing.channel,
                format!("couldn't read files folder: {error}"),
            );
        }
    }
}
