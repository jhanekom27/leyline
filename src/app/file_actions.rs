use super::*;

impl AppState {
    /// Handles `/save [hash-prefix]`: `arg` is everything after `/save `
    /// (already trimmed). With a prefix, it names (a prefix of) a file
    /// previously shared in any joined channel -- see `resolve_attachment`,
    /// which mirrors `/alias`'s hex-prefix resolution for a consistent UX.
    /// With none, it takes the newest file another peer shared in the
    /// active channel (`newest_shared_file`), mirroring a bare `/reply`'s
    /// "most recent message" default -- name an older file by its prefix.
    /// Either way the file goes through the same `InputAction::SaveFile`,
    /// asking its sender and every other peer known where it was shared
    /// (`providers_for`), not only the sender, and landing in the folder of
    /// the channel it was shared in (`channel_of_file`).
    pub(super) fn run_save(&mut self, arg: &str) -> Option<InputAction> {
        let file = if arg.is_empty() {
            self.newest_shared_file()
        } else {
            self.resolve_attachment(arg)
        };
        match file {
            Ok((hash, filename, sender)) => {
                let channel = self.channel_of_file(hash).to_string();
                Some(self.save_request(channel, hash, filename, sender))
            }
            Err(err) => {
                self.push_system(err);
                None
            }
        }
    }

    /// Builds the `InputAction::SaveFile` that fetches the file `hash` into
    /// `channel`'s folder, and leaves the `saving ...` notice -- the one
    /// place that is done, so `/save` and `Enter` in the files overlay can't
    /// differ. `filename` and `sender` are as `resolve_attachment` gives
    /// them. The file is asked of its sender first and then of every other
    /// peer known where it was shared (`providers_for`), and goes into the
    /// folder (`files::channel_dir`) of `channel`, a stored name. Someone
    /// asked for this file and can see what it is, so it is a
    /// `SaveMode::Manual` save: no limit on its size.
    pub(super) fn save_request(
        &mut self,
        channel: String,
        hash: Hash,
        filename: String,
        sender: [u8; 32],
    ) -> InputAction {
        let providers = self.providers_for(hash, sender);
        let dir = self.folder_dir(&channel);
        self.push_system(format!("saving {filename}..."));
        InputAction::SaveFile(SaveRequest {
            channel,
            hash,
            filename,
            providers,
            dir,
            mode: SaveMode::Manual,
        })
    }

    /// Decides what to do about a file that has just arrived live in
    /// `channel` (a stored name). main.rs calls this for each message newly
    /// accepted off the network (`handle_net_event`), and for nothing else:
    /// history loaded at startup and history backfilled from a peer never
    /// come here, so a file shared before this session, while auto-save was
    /// off, or before this device joined is never fetched on its own.
    /// `already_saved` says whether the channel's folder already holds a copy
    /// of a file, which app.rs can't look at.
    ///
    /// Only a file from another device, in a channel with auto-save on
    /// (`/autosave`), is any concern of this; for anything else it says
    /// nothing. For one that is, it says why and stops when the sender isn't
    /// trusted (`is_trusted_peer`), the file is over the limit
    /// (`AUTOSAVE_MAX_FILE_SIZE`) or the channel already holds a copy, and
    /// otherwise returns the `SaveRequest`. Every notice goes to `channel`'s
    /// transcript, wherever the user is looking, and points to the `/save`
    /// that fetches the file by hand.
    ///
    /// `message.sender` is only what the sender wrote: nothing signs a chat
    /// message, so anyone in the channel can claim to be someone else. It
    /// decides whether to try at all and is not what keeps strangers out --
    /// the providers are. The file is only ever asked of peers that are
    /// themselves trusted, who are known by the connection to them rather
    /// than by what a message says, so a forged share of a file that only a
    /// stranger has fetches nothing. The download is refused, too, if the
    /// file turns out bigger than its sender said (`SaveMode::Automatic`).
    pub fn auto_save(
        &mut self,
        channel: &str,
        message: &ChatMessage,
        already_saved: impl Fn(Hash) -> bool,
    ) -> Option<SaveRequest> {
        let file = message.attachment.as_ref()?;
        if message.sender == self.self_id || !self.autosave.contains(channel) {
            return None;
        }
        let hash = file.hash;
        let by_hand = format!("{} to download it yourself", save_hint(hash));
        if !self.is_trusted_peer(&message.sender) {
            self.push_system_to(
                channel,
                format!(
                    "not auto-saving {} from {}: auto-save only takes files from your own devices and the peers you have aliased -- {by_hand}",
                    file.filename,
                    self.display_name(&message.sender)
                ),
            );
            return None;
        }
        let max_bytes = self.autosave_limit_for(channel).max_bytes();
        if file.size > max_bytes {
            self.push_system_to(
                channel,
                format!(
                    "not auto-saving {} ({}): over the {} auto-save limit -- {by_hand}",
                    file.filename,
                    human_size(file.size),
                    human_size(max_bytes)
                ),
            );
            return None;
        }
        if already_saved(hash) {
            self.push_system_to(
                channel,
                format!(
                    "not auto-saving {}: this channel already has a saved copy of it",
                    file.filename
                ),
            );
            return None;
        }
        if !self.auto_saving.insert((channel.to_string(), hash)) {
            // The same file is being fetched right now: its notice will come.
            return None;
        }
        let providers = self
            .providers_for(hash, message.sender)
            .into_iter()
            .filter(|id| self.is_trusted_peer(id))
            .collect();
        Some(SaveRequest {
            channel: channel.to_string(),
            hash,
            filename: file.filename.clone(),
            providers,
            dir: self.folder_dir(channel),
            mode: SaveMode::Automatic { max_bytes },
        })
    }

    /// Whether files from `id` may be saved without anyone asking: one of our
    /// own devices -- certified under our own user key, so it resolves
    /// (`canonical_id`) to `self_user_id`; a device forgotten with
    /// `/forget-device` doesn't -- or a peer we have given an alias
    /// (`petnames`, keyed by the same resolved identity, so an alias set on
    /// any one of a person's devices covers them all). The one answer both
    /// halves of auto-save go by (`auto_save`).
    pub(super) fn is_trusted_peer(&self, id: &[u8; 32]) -> bool {
        let identity = self.canonical_id(id);
        identity == self.self_user_id || self.petnames.contains_key(&identity)
    }

    /// Handles `/folder [path]`: `arg` is everything after `/folder `
    /// (already trimmed). With no path, says where received files go: the
    /// active channel's folder -- named from its stored name, so `/rename`
    /// and `/alias` don't move it -- and the files root it is under. With
    /// one, moves the files root there: strips one layer of quotes, as
    /// `/send` does, and asks the caller to do it
    /// (`InputAction::SetFilesRoot`), since whether the path will do depends
    /// on the filesystem, which app.rs never touches.
    pub(super) fn run_folder(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            let channel = self.active().name.clone();
            self.push_system(format!(
                "files from {} are saved in {} -- every channel's folder is under {}, which /folder <path> moves",
                self.channel_heading(&channel),
                self.folder_dir(&channel).display(),
                self.files_root.display()
            ));
            return None;
        }
        Some(InputAction::SetFilesRoot(strip_quotes(arg).to_string()))
    }

    /// Handles `/maxsize [<size>|off|default]`: `arg` is everything after
    /// `/maxsize ` (already trimmed). With nothing, says what the limit on
    /// the size of a shared file is and how to change it, as a bare `/folder`
    /// says where files go. With something, reads it
    /// (`SizeLimit::parse`, the one place it is validated: what it refuses
    /// leaves the limit as it was), applies it at once, and asks the caller
    /// to remember it (`InputAction::SetSizeLimit`), as `/bell` does. There
    /// is nothing else to tell: every route a file takes to being shared
    /// checks `size_limit` in main.rs.
    pub(super) fn run_maxsize(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system(self.size_limit_notice());
            return None;
        }
        match SizeLimit::parse(arg) {
            Ok(limit) => {
                self.size_limit = limit;
                self.push_system(self.size_limit_notice());
                Some(InputAction::SetSizeLimit(limit))
            }
            Err(message) => {
                self.push_system(message);
                None
            }
        }
    }

    /// Handles `/autosave [on|off|limit <size|default>]`: `arg` is everything after `/autosave `
    /// (already trimmed). With nothing, says whether auto-save is on for the
    /// active channel -- it is off until turned on, for every channel -- and
    /// what it would do. With `on` or `off`, switches it for the active
    /// channel (a group channel, `/msg` DM or `/dm` chat alike) and asks the
    /// caller to remember it (`InputAction::SetAutoSave`), but only if that
    /// changed anything: asking for what already holds writes nothing.
    /// Anything else is usage, and changes nothing.
    pub(super) fn run_autosave(&mut self, arg: &str) -> Option<InputAction> {
        if let Some(size) = arg.strip_prefix("limit ") {
            return match AutoSaveLimit::parse(size) {
                Ok(limit) => {
                    let channel = self.active().name.clone();
                    let override_limit = match limit {
                        AutoSaveLimit::Default => None,
                        limit => Some(limit),
                    };
                    match override_limit {
                        Some(limit) => {
                            self.autosave_limits.insert(channel.clone(), limit);
                        }
                        None => {
                            self.autosave_limits.remove(&channel);
                        }
                    }
                    self.push_system(self.autosave_limit_notice());
                    Some(InputAction::SetChannelAutoSaveLimit {
                        channel,
                        limit: override_limit,
                    })
                }
                Err(message) => {
                    self.push_system(message);
                    None
                }
            };
        }
        let channel = self.active().name.clone();
        let on = match arg {
            "" => {
                self.push_system(self.autosave_notice(&channel));
                return None;
            }
            "on" => true,
            "off" => false,
            _ => {
                self.push_system("usage: /autosave [on|off|limit <size|default>]");
                return None;
            }
        };
        let changed = if on {
            self.autosave.insert(channel.clone())
        } else {
            self.autosave.remove(&channel)
        };
        self.push_system(self.autosave_notice(&channel));
        changed.then_some(InputAction::SetAutoSave { channel, on })
    }

    /// Handles `/outbox [on|off]`. The setting is only applied once the
    /// runtime has created and snapshot the folder successfully.
    pub(super) fn run_outbox(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        let on = match arg {
            "" => {
                self.push_system(self.outbox_notice(&channel));
                return None;
            }
            "on" => true,
            "off" => false,
            _ => {
                self.push_system("usage: /outbox [on|off]");
                return None;
            }
        };
        if self.outbox.contains(&channel) == on {
            self.push_system(self.outbox_notice(&channel));
            return None;
        }
        Some(InputAction::SetOutbox { channel, on })
    }
}
