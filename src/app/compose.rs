use super::*;

impl AppState {
    /// Handles `/paste` (and the `Ctrl+V`/`Cmd+V` keybinding in
    /// `handle_key`): shares whatever's currently on the OS clipboard with
    /// the active channel. Takes no argument -- like `/invite`, it always
    /// acts on the active channel. app.rs never touches the clipboard
    /// itself (see this module's doc comment); the caller
    /// (`net::Net::paste`) reads it and reports back what it found (a
    /// file, an image, or plain text) as a `NetEvent`. Pushes an immediate
    /// breadcrumb since reading the clipboard and importing its contents
    /// happens in the background and may take a moment, mirroring
    /// `run_send`.
    pub(super) fn run_paste(&mut self) -> Option<InputAction> {
        self.push_system("pasting from clipboard...");
        Some(InputAction::Paste(self.active().name.clone()))
    }

    /// Handles `/msg <alias-or-hex-prefix> [text]`: `arg` is everything
    /// after `/msg ` (already trimmed), split on the first space into a
    /// target (resolved via `resolve_msg_target`) and an optional message
    /// to send immediately, mirroring `/reply [text]`'s shape. If a DM
    /// channel is already joined for that peer (`dm_channels`, see
    /// `crate::dm_registry`), just switches to it -- else picks a fresh
    /// local name (`unique_dm_channel_name`) and returns an
    /// `InputAction::StartDm` for the caller to create it (see
    /// `net::Net::join` and `NetEvent::Joined`/`DmJoined`). A recorded DM
    /// channel that isn't currently joined (e.g. left outside of `/leave`)
    /// is treated the same as none at all -- `activate_existing_channel`
    /// returning `false` falls through to creating a new one.
    pub(super) fn run_msg(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system("usage: /msg <alias-or-hex-prefix> [text]");
            return None;
        }
        let (target, text) = match arg.split_once(' ') {
            Some((target, rest)) => (target, Some(rest.trim()).filter(|s| !s.is_empty())),
            None => (arg, None),
        };
        let id = match self.resolve_msg_target(target) {
            Ok(id) => id,
            Err(err) => {
                self.push_system(err);
                return None;
            }
        };

        if let Some(name) = self.dm_channels.get(&id).cloned()
            && self.activate_existing_channel(&name)
        {
            self.push_system(format!(
                "switched to your DM with {}",
                self.display_name(&id)
            ));
            return text.map(|text| self.send_text(text, None));
        }

        let channel = self.unique_dm_channel_name(&id);
        self.push_system(format!(
            "starting a DM with {} in #{channel} -- share the invite once it's ready",
            self.display_name(&id)
        ));
        Some(InputAction::StartDm {
            peer: id,
            channel,
            text: text.map(str::to_string),
        })
    }

    /// Handles `/dm <alias|hex-prefix|endpoint-id> [text]`: a true
    /// point-to-point conversation, with no gossip channel, room secret,
    /// or invite ticket involved (see `direct.rs`) -- `/msg` is the
    /// gossip-based equivalent. `arg` is everything after `/dm ` (already
    /// trimmed), split the same way `run_msg` splits its argument.
    /// Resolves the target via `resolve_dm_target`, which -- unlike
    /// `resolve_msg_target` -- also accepts a full endpoint id for a peer
    /// never otherwise observed, so two people who already know each
    /// other's id can start talking without having shared a channel
    /// first. If a point-to-point tab is already open for that peer
    /// (`direct_channels`), just switches to it -- else returns
    /// `InputAction::OpenDirect` for the caller (main.rs) to ensure the
    /// connection and create the tab, since unlike `run_msg`'s gossip
    /// subscribe, there's nothing here worth doing synchronously ahead of
    /// that (`net::Net::open_direct` is itself synchronous).
    pub(super) fn run_dm(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system("usage: /dm <alias|hex-prefix|endpoint-id> [text]");
            return None;
        }
        let (target, text) = match arg.split_once(' ') {
            Some((target, rest)) => (target, Some(rest.trim()).filter(|s| !s.is_empty())),
            None => (arg, None),
        };
        let id = match self.resolve_dm_target(target) {
            Ok(id) => id,
            Err(err) => {
                self.push_system(err);
                return None;
            }
        };
        if id == self.self_id {
            self.push_system("you can't start a point-to-point chat with yourself");
            return None;
        }

        if let Some(name) = self.direct_channels.get(&id).cloned()
            && self.activate_existing_channel(&name)
        {
            self.push_system(format!(
                "switched to your point-to-point chat with {}",
                self.display_name(&id)
            ));
            return text.map(|text| self.send_text(text, None));
        }

        self.push_system(format!(
            "starting a point-to-point chat with {}...",
            self.display_name(&id)
        ));
        Some(InputAction::OpenDirect {
            peer: id,
            text: text.map(str::to_string),
        })
    }

    /// Resolves a `/msg` target: an exact (case-insensitive) local pet
    /// name first -- checked against every saved pet name, not just
    /// `known_ids()`, since you should be able to `/msg` someone you've
    /// aliased even if they aren't observed anywhere right now -- else a
    /// hex-prefix via `resolve_id` (which _is_ scoped to `known_ids()`,
    /// since there's nothing else to match a raw prefix against). Errors
    /// mirror `resolve_id`'s for a consistent UX.
    pub(super) fn resolve_msg_target(&self, target: &str) -> Result<[u8; 32], String> {
        let matches: Vec<[u8; 32]> = self
            .petnames
            .iter()
            .filter(|(_, name)| name.eq_ignore_ascii_case(target))
            .map(|(id, _)| *id)
            .collect();
        match matches.as_slice() {
            [id] => Ok(*id),
            [] => self.resolve_id(target),
            _ => Err(format!(
                "'{target}' matches multiple aliased peers, use a hex prefix instead"
            )),
        }
    }

    /// Resolves a `/dm` target: `resolve_msg_target`'s pet-name/hex-prefix
    /// resolution first, falling back to parsing `target` itself as a
    /// full, not-necessarily-already-known endpoint id (exactly 64 hex
    /// characters) if that fails. This fallback is what lets `/dm` reach
    /// a peer who's never appeared in any joined channel or been
    /// `/alias`ed -- the whole point of a point-to-point connection being
    /// usable the moment two people already know each other's endpoint
    /// id, with nothing else shared yet.
    pub(super) fn resolve_dm_target(&self, target: &str) -> Result<[u8; 32], String> {
        match self.resolve_msg_target(target) {
            Ok(id) => Ok(id),
            Err(err) => decode_hex_id(target).ok_or(err),
        }
    }

    /// Picks a not-currently-joined local channel name for a new DM with
    /// `id`: `dm-<label>` (their pet name if set, else their hex prefix),
    /// suffixed with `-2`, `-3`, ... on the rare chance that's already
    /// taken by an unrelated channel.
    pub(super) fn unique_dm_channel_name(&self, id: &[u8; 32]) -> String {
        let label = self
            .petnames
            .get(id)
            .cloned()
            .unwrap_or_else(|| hex_prefix(id));
        let base = format!("dm-{label}");
        if !self.channels.iter().any(|c| c.name == base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|name| !self.channels.iter().any(|c| &c.name == name))
            .expect("an unbounded suffix sequence always finds a free name")
    }

    /// Resolves a hex-prefix (case-insensitive, as typed after `/alias`)
    /// to exactly one id in `known_ids`. Errors with a user-facing message
    /// if the prefix isn't valid hex, matches no known id, or matches more
    /// than one -- this is what makes `/alias` "do nothing for a peer you
    /// haven't met yet" rather than guessing.
    pub(super) fn resolve_id(&self, prefix: &str) -> Result<[u8; 32], String> {
        let prefix = prefix.to_lowercase();
        let valid_hex = !prefix.is_empty()
            && prefix.len() <= 64
            && prefix.chars().all(|c| c.is_ascii_hexdigit());
        if !valid_hex {
            return Err(format!("invalid hex prefix: {prefix}"));
        }
        let matches: Vec<[u8; 32]> = self
            .known_ids()
            .into_iter()
            .filter(|id| hex_id(id).starts_with(&prefix))
            .collect();
        match matches.as_slice() {
            [] => Err(format!("no known peer matches '{prefix}'")),
            [id] => Ok(*id),
            _ => Err(format!(
                "'{prefix}' matches multiple peers, use a longer prefix"
            )),
        }
    }

    /// Every endpoint id `AppState` has actually observed: everyone
    /// currently online in any joined channel, plus the sender of any
    /// chat message in any channel's transcript (including messages
    /// loaded from history) -- i.e. everyone `/alias` could plausibly
    /// name, whether or not they're online right now. Excludes our own
    /// id, and is deduplicated.
    pub(super) fn known_ids(&self) -> Vec<[u8; 32]> {
        let mut ids: Vec<[u8; 32]> = Vec::new();
        for channel in &self.channels {
            ids.extend(channel.peers.iter().copied());
            for line in &channel.messages {
                if let TranscriptLine::Chat(message) = line {
                    ids.push(message.sender);
                }
            }
        }
        ids.retain(|id| *id != self.self_id);
        ids.sort();
        ids.dedup();
        ids
    }

    /// Resolves a hex-prefix (case-insensitive, as typed after `/save`) to
    /// exactly one attachment in `known_attachments`, returning its hash,
    /// filename, and original sender (the first peer `/save` asks for it --
    /// see `providers_for`). Mirrors `resolve_id`'s errors for a
    /// consistent UX: invalid hex, no match, or an ambiguous prefix.
    pub(super) fn resolve_attachment(
        &self,
        prefix: &str,
    ) -> Result<(Hash, String, [u8; 32]), String> {
        let prefix = prefix.to_lowercase();
        let valid_hex = !prefix.is_empty()
            && prefix.len() <= 64
            && prefix.chars().all(|c| c.is_ascii_hexdigit());
        if !valid_hex {
            return Err(format!("invalid hash prefix: {prefix}"));
        }
        let matches: Vec<(Hash, String, [u8; 32])> = self
            .known_attachments()
            .into_iter()
            .filter(|(hash, _, _)| hash.to_hex().starts_with(&prefix))
            .collect();
        match matches.as_slice() {
            [] => Err(format!("no known file matches '{prefix}'")),
            [one] => Ok(one.clone()),
            _ => Err(format!(
                "'{prefix}' matches multiple files, use a longer prefix"
            )),
        }
    }

    /// Every file attachment `AppState` has seen in any joined channel's
    /// transcript (including history loaded at startup), deduplicated by
    /// hash -- i.e. everything `/save` could plausibly resolve. Mirrors
    /// `known_ids`'s shape.
    pub(super) fn known_attachments(&self) -> Vec<(Hash, String, [u8; 32])> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for channel in &self.channels {
            if let Some(files) = self.shared_files.get(&channel.name) {
                for shared in files {
                    if seen.insert(shared.file.hash) {
                        result.push((
                            shared.file.hash,
                            shared.file.filename.clone(),
                            shared.sender,
                        ));
                    }
                }
            }
        }
        result
    }

    /// The newest file another peer shared in the active channel -- what a
    /// bare `/save` fetches, as a bare `/reply` answers the newest message
    /// -- as `resolve_attachment` returns one: its hash, filename, and its
    /// sender. "Another peer" is anyone but this device
    /// (`self_id`), the test `ui::attachment_line` uses to give a file its
    /// `/save <hash>` hint, so this is always the newest file that shows
    /// one -- including a file from one of our own other devices, which
    /// isn't on this one. Looks through the channel's loaded transcript,
    /// like `known_attachments`, not `Channel::visible_chat_ids`: an open
    /// thread or search narrows what `/reply` can target, but the file just
    /// shared belongs to the channel, not to the view. Errors with a
    /// user-facing message if there is none.
    pub(super) fn newest_shared_file(&self) -> Result<(Hash, String, [u8; 32]), String> {
        self.shared_files
            .get(&self.active().name)
            .into_iter()
            .flatten()
            .find(|shared| shared.sender != self.self_id)
            .map(|shared| {
                (
                    shared.file.hash,
                    shared.file.filename.clone(),
                    shared.sender,
                )
            })
            .ok_or_else(|| "no file from another peer to save in this channel".to_string())
    }

    /// The channel a saved file belongs to -- the one whose folder it goes
    /// in (`files::channel_dir`), as its stored name: the active channel if
    /// its transcript carries the file, else the first joined channel that
    /// does. A file shared in several channels is saved to the one you are
    /// looking at when it can be; `/save <prefix>` can also name a file from
    /// a channel you aren't in, and then it goes where it was shared.
    ///
    /// # Panics
    /// Panics if no joined channel carries `hash` -- the callers have just
    /// found the file in a transcript (`resolve_attachment`,
    /// `newest_shared_file`).
    pub(super) fn channel_of_file(&self, hash: Hash) -> &str {
        let active = self.active().name.as_str();
        self.channels
            .iter()
            .map(|channel| channel.name.as_str())
            .filter(|channel| {
                self.shared_files
                    .get(*channel)
                    .is_some_and(|files| files.iter().any(|shared| shared.file.hash == hash))
            })
            .find(|channel| *channel == active)
            .or_else(|| {
                self.channels
                    .iter()
                    .map(|channel| channel.name.as_str())
                    .find(|channel| {
                        self.shared_files.get(*channel).is_some_and(|files| {
                            files.iter().any(|shared| shared.file.hash == hash)
                        })
                    })
            })
            .expect("a file /save resolved was found in shared-file metadata")
    }

    /// Who `/save` asks for the file `hash`, best bet first: its `sender`;
    /// then the peers online right now in the channels that carry it
    /// (`Channel::peers`); then everyone who has posted in those channels.
    /// Anyone who saved the file serves it from their own blob store (see
    /// `BackfillStore::fetch_file`), so a file does not depend on its
    /// sender being online -- but nothing says who saved it, so every peer
    /// we know of is a candidate, the ones reachable now before the ones
    /// that are probably gone. Posters come after the online peers because
    /// gossip only keeps a handful of direct neighbors: someone who has the
    /// file may be online without being one of them.
    ///
    /// Never this device -- except as the sender, when the file is already
    /// in our own blob store and needs no one -- and each peer once. A
    /// channel carries the file if its loaded transcript has a message with
    /// this hash; a file shared in several asks all of them, so the answer
    /// does not depend on which channel `/save` happened to resolve it in.
    pub(super) fn providers_for(&self, hash: Hash, sender: [u8; 32]) -> Vec<[u8; 32]> {
        let channels: Vec<&Channel> = self
            .channels
            .iter()
            .filter(|channel| {
                self.shared_files
                    .get(&channel.name)
                    .is_some_and(|files| files.iter().any(|shared| shared.file.hash == hash))
            })
            .collect();
        let online = channels
            .iter()
            .flat_map(|channel| channel.peers.iter().copied());
        let posters = channels.iter().flat_map(|channel| {
            self.shared_files
                .get(&channel.name)
                .into_iter()
                .flatten()
                .map(|shared| shared.sender)
        });
        let live_posters = channels.iter().flat_map(|channel| {
            channel.messages.iter().filter_map(|line| match line {
                TranscriptLine::Chat(message) => Some(message.sender),
                TranscriptLine::System(_) => None,
            })
        });

        let mut providers = vec![sender];
        for id in online.chain(posters).chain(live_posters) {
            if id != self.self_id && !providers.contains(&id) {
                providers.push(id);
            }
        }
        providers
    }

    /// Whether keys -- and pastes -- currently go to the message box. False
    /// while a live pairing redeem is awaiting confirmation, a reply or
    /// thread pick is in progress, or the `Ctrl+P` switcher, the file picker
    /// or the files overlay is open: each takes over input ahead of normal
    /// typing (see `handle_key`). main.rs only treats a pasted file path as
    /// a dropped file while this is true, since staging files while the user
    /// is in the middle of something else would put them somewhere they are
    /// not looking.
    pub fn is_composing(&self) -> bool {
        self.pending_redeem.is_none()
            && self.picking_reply.is_none()
            && self.picking_thread.is_none()
            && self.switching.is_none()
            && self.command_picker.is_none()
            && self.viewing_peers.is_none()
            && self.picking_file.is_none()
            && self.viewing_files.is_none()
    }

    /// Inserts a terminal bracketed-paste's full text at the cursor as one
    /// atomic edit (see main.rs's `Event::Paste` handling, enabled via
    /// `EnableBracketedPaste`). Without bracketed paste, a multi-line
    /// paste instead arrives as a fast burst of ordinary key events, so an
    /// embedded newline mid-paste would hit `submit_input` partway through
    /// -- prematurely sending a partial line or running a stray
    /// `/command`. `paste_text` never submits, so a pasted newline just
    /// becomes a real line break (`insert_newline`) instead. A paste that is
    /// really a dropped file never gets here -- main.rs's `handle_paste`
    /// diverts it first.
    pub fn paste_text(&mut self, text: &str) {
        // An open switcher owns input: a paste filters it rather than
        // landing, unseen, in the draft underneath. Its query is one line,
        // so line breaks and other control characters are dropped.
        if self.switching.is_some() {
            self.edit_switcher_query(|query| {
                query.extend(text.chars().filter(|c| !c.is_control()));
            });
            return;
        }
        if self.command_picker.is_some() {
            self.edit_command_picker_query(|query| {
                query.extend(text.chars().filter(|c| !c.is_control()));
            });
            return;
        }
        if self.viewing_peers.is_some() {
            return;
        }
        // So does an open file picker, whose filter is one line too.
        if let Some(picker) = &mut self.picking_file {
            picker.edit_query(|query| {
                query.extend(text.chars().filter(|c| !c.is_control()));
            });
            return;
        }
        // And an open files overlay, with the same one-line filter.
        if self.viewing_files.is_some() {
            self.edit_files_query(|query| {
                query.extend(text.chars().filter(|c| !c.is_control()));
            });
            return;
        }
        let channel = self.active_mut();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                // `\r\n` counts as a single line break, not two -- without
                // this, a Windows-style multi-line paste would insert an
                // extra blank line between paragraphs.
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    channel.insert_newline();
                }
                '\n' => channel.insert_newline(),
                c => channel.insert_char(c),
            }
        }
    }
}
