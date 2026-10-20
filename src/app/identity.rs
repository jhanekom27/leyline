use super::*;

impl AppState {
    /// Places a locally-authored message (currently only a finished
    /// `/send`, via `NetEvent::FileReady`) into `channel`'s transcript,
    /// deduped the same way an incoming message is -- called by main.rs
    /// once it has broadcast and persisted the message, since its content
    /// (the file's hash) isn't known until the background import
    /// finishes. Unlike `handle_net_event`'s `Received` case, never marks
    /// the channel unread -- there's no point notifying yourself about
    /// your own successfully sent file. A no-op if `channel` isn't
    /// currently joined.
    pub fn record_sent_message(&mut self, channel: &str, message: ChatMessage) {
        if let Some(file) = &message.attachment {
            self.record_shared_file(
                channel,
                SharedFile {
                    message_id: message.id,
                    file: file.clone(),
                    sender: message.sender,
                    ts_unix_ms: message.ts_unix_ms,
                },
            );
        }
        if let Some(channel) = self.channel_mut(channel)
            && channel.remember_seen(message.id)
        {
            channel.push(TranscriptLine::Chat(message));
        }
    }

    /// Stages `item` -- already vetted by `files::stage_file`/
    /// `stage_image` -- on `channel`, ready for review and `Enter`. Called
    /// by main.rs when a clipboard read (`net::NetEvent::ClipboardFiles`/
    /// `ClipboardImage`) comes back, which may be after the user has moved
    /// to another tab; the event names the channel the paste was for, so
    /// the attachment lands there rather than wherever focus is now. A
    /// no-op if `channel` isn't currently joined.
    pub fn stage_attachment(&mut self, channel: &str, item: StagedAttachment) {
        if let Some(channel) = self.channel_mut(channel) {
            channel.staged.push(item);
        }
    }

    /// Composes a `ChatMessage` as if `text` had been typed and sent by us
    /// -- used for `/paste`'s clipboard-text fallback (see
    /// `net::NetEvent::ClipboardText`), which arrives asynchronously from
    /// the background clipboard read rather than through
    /// `handle_key`/`submit_input`. The caller still needs to broadcast,
    /// persist, and display the result exactly like any other
    /// locally-authored message (see `record_sent_message`). Never a
    /// reply -- this flow doesn't go through the reply-picking UI.
    pub fn compose_own_message(&self, text: &str) -> ChatMessage {
        compose_message(self.self_id, text, None)
    }

    /// Displays a sender as "you" for our own id, or for *any* peer
    /// certified under our own `self_user_id` -- i.e. one of our own other
    /// devices, paired via `/pair` (see `canonical_id`) -- since from a
    /// human's perspective a message or presence from your other device is
    /// still just you, not a second stranger; their pet name if one has
    /// been assigned via `/alias` (keyed by their resolved `canonical_id`,
    /// not necessarily the literal `sender`, so an alias set against any
    /// one of a person's devices applies to all of them once their
    /// certificates are known -- see `canonical_id`); else their last
    /// broadcast nickname from `/nick`, suffixed with their hex prefix
    /// since a nickname is spoofable and not unique (features.md's
    /// "Broadcast nicknames"); or, absent both, a shortened hex id on its
    /// own.
    pub fn display_name(&self, sender: &[u8; 32]) -> String {
        if *sender == self.self_id {
            return "you".to_string();
        }
        let id = self.canonical_id(sender);
        if id == self.self_user_id {
            return "you".to_string();
        }
        if let Some(name) = self.petnames.get(&id) {
            name.clone()
        } else if let Some(nickname) = self.nicknames.get(&id) {
            format!("{nickname} ({})", hex_prefix(sender))
        } else {
            hex_prefix(sender)
        }
    }

    /// Resolves `id` to the user identity it belongs to, if a verified
    /// `crate::message::DeviceCert` has been seen for it (`device_to_user`)
    /// -- otherwise `id` unchanged, which is also correct for our own id
    /// and for any peer we have no cert for (an older build, or one not
    /// yet verified). `display_name`, `/who`'s grouping
    /// (`grouped_who_lines`), and `/alias` (`run_alias`) all resolve
    /// through this first, so two of one person's devices collapse onto
    /// whichever single petname/nickname has been recorded for that
    /// person, instead of needing one recorded per device. Note that this
    /// never maps our own `self_id` to `self_user_id` -- nothing ever
    /// sends us a certificate for our own device id, since `announce_
    /// device` only ever broadcasts outward -- so callers that need to
    /// recognize *our own* sibling devices (`display_name`, `unique_
    /// peer_ids`) compare a peer's `canonical_id` against `self_user_id`
    /// directly instead of relying on this method alone.
    ///
    /// A forgotten device (`/forget-device`, see `forgotten`) is always
    /// treated as if no certificate were known for it, regardless of what
    /// `device_to_user` still remembers -- forgetting takes effect
    /// immediately, not just for future certificates.
    pub fn canonical_id(&self, id: &[u8; 32]) -> [u8; 32] {
        if self.forgotten.contains(id) {
            return *id;
        }
        self.device_to_user.get(id).copied().unwrap_or(*id)
    }

    /// Deduplicates `ids` by `canonical_id`, keeping the first device seen
    /// for each distinct person and preserving their original relative
    /// order. The sidebar uses this grouping, while `/who` deliberately
    /// keeps its device rows separate. Also excludes any peer certified under
    /// our own `self_user_id` entirely, rather than merging it into some
    /// other group: one of our own other devices is already represented
    /// by the sidebar's separate "you" row (`ui::render_peers`), so
    /// listing it again here would just be the same person shown twice
    /// under two different labels. A peer with no known certificate is
    /// simply its own one-member group, so this changes nothing for it.
    #[cfg(test)]
    pub fn unique_peer_ids(&self, ids: &[[u8; 32]]) -> Vec<[u8; 32]> {
        let mut seen = HashSet::new();
        ids.iter()
            .filter(|id| self.canonical_id(id) != self.self_user_id)
            .filter(|id| seen.insert(self.canonical_id(id)))
            .copied()
            .collect()
    }

    /// Records a current observation of `id` carried by a valid device
    /// certificate. Unlike direct-neighbor presence, this can arrive from a
    /// remote channel member relayed through the gossip mesh, so it expires
    /// after `PEER_ACTIVITY_TTL_MS` rather than waiting for `NeighborDown`.
    /// The runtime owns the clock and supplies one observation timestamp.
    pub fn record_peer_activity(&mut self, channel: &str, id: [u8; 32], observed_unix_ms: u64) {
        if let Some(channel) = self.channel_mut(channel) {
            channel.recent_peers.insert(id, observed_unix_ms);
        }
    }

    /// Drops remote-device observations that have missed three certificate
    /// heartbeats. Returns whether any channel changed so the runtime can
    /// redraw the sidebar and `/who` without waiting for a network event.
    pub fn expire_peer_activity(&mut self, now_unix_ms: u64) -> bool {
        let mut changed = false;
        for channel in &mut self.channels {
            let before = channel.recent_peers.len();
            channel.recent_peers.retain(|_, seen_unix_ms| {
                now_unix_ms.saturating_sub(*seen_unix_ms) < PEER_ACTIVITY_TTL_MS
            });
            changed |= channel.recent_peers.len() != before;
        }
        changed
    }

    /// The active channel's online endpoint ids. Direct neighbors retain
    /// their event-driven status; recently observed remote devices follow in
    /// a deterministic order so their derived user row stays stable.
    fn active_peer_ids(&self) -> Vec<[u8; 32]> {
        let channel = self.active();
        let mut ids = channel.peers.clone();
        let mut remote: Vec<_> = channel.recent_peers.keys().copied().collect();
        remote.sort_unstable();
        for id in remote {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids
    }

    /// The active channel's peers for the sidebar: currently-online people
    /// first and remembered offline people after them. Multiple endpoint ids
    /// certified for one person collapse to one row; any active sibling
    /// keeps that person online, and otherwise the newest observation wins.
    pub fn peer_presence(&self) -> Vec<PeerPresence> {
        let channel = self.active();
        let mut peers = Vec::new();

        for id in self.active_peer_ids() {
            let canonical = self.canonical_id(&id);
            if canonical == self.self_user_id
                || peers
                    .iter()
                    .any(|peer: &PeerPresence| self.canonical_id(&peer.id) == canonical)
            {
                continue;
            }
            peers.push(PeerPresence {
                id,
                last_seen_unix_ms: None,
            });
        }

        let mut offline: Vec<_> = channel.last_seen.iter().collect();
        offline.sort_by_key(|(id, seen)| (std::cmp::Reverse(**seen), **id));
        for (id, seen_unix_ms) in offline {
            let canonical = self.canonical_id(id);
            if canonical == self.self_user_id {
                continue;
            }
            match peers
                .iter_mut()
                .find(|peer| self.canonical_id(&peer.id) == canonical)
            {
                Some(peer) if peer.last_seen_unix_ms.is_none() => {}
                Some(peer) if peer.last_seen_unix_ms < Some(*seen_unix_ms) => {
                    *peer = PeerPresence {
                        id: *id,
                        last_seen_unix_ms: Some(*seen_unix_ms),
                    };
                }
                Some(_) => {}
                None => peers.push(PeerPresence {
                    id: *id,
                    last_seen_unix_ms: Some(*seen_unix_ms),
                }),
            }
        }

        peers
    }

    /// The active channel's raw device records for `/who`. This deliberate
    /// diagnostic view neither collapses canonical identities nor excludes a
    /// paired sibling device; callers add this device's own row separately.
    pub fn who_presence(&self) -> Vec<PeerPresence> {
        let channel = self.active();
        let active = self.active_peer_ids();
        let mut seen: HashSet<_> = active.iter().copied().collect();
        let mut peers: Vec<_> = active
            .into_iter()
            .map(|id| PeerPresence {
                id,
                last_seen_unix_ms: None,
            })
            .collect();

        let mut offline: Vec<_> = channel.last_seen.iter().collect();
        offline.sort_by_key(|(id, seen_unix_ms)| (std::cmp::Reverse(**seen_unix_ms), **id));
        for (id, seen_unix_ms) in offline {
            if seen.insert(*id) {
                peers.push(PeerPresence {
                    id: *id,
                    last_seen_unix_ms: Some(*seen_unix_ms),
                });
            }
        }
        peers
    }

    /// The newest peer version we've heard (via `NetEvent::Version`) that's
    /// actually newer than ours, if any -- used by `ui::render_header`'s
    /// persistent indicator and `/version`'s own report. Recomputed on
    /// demand rather than cached, since `peer_versions` is small (bounded
    /// by distinct peers ever seen, like `nicknames`).
    pub fn newest_known_update(&self) -> Option<(&str, &str)> {
        self.peer_versions
            .values()
            .filter(|(candidate, _)| version::is_newer(candidate))
            .max_by_key(|(candidate, _)| version::parse(candidate))
            .map(|(candidate, hash)| (candidate.as_str(), hash.as_str()))
    }

    /// Appends a local-only system notice to the active channel's
    /// transcript (invite output, command errors) -- never sent over the
    /// network.
    pub fn push_system(&mut self, text: impl Into<String>) {
        self.active_mut().push(TranscriptLine::System(text.into()));
    }

    /// Appends a local-only system notice to `channel`'s transcript, wherever
    /// the user happens to be looking -- for what concerns one channel and
    /// happens without anyone having asked just now (auto-save). A no-op if
    /// `channel` isn't joined, which is only the case once it has been left.
    pub fn push_system_to(&mut self, channel: &str, text: impl Into<String>) {
        if let Some(channel) = self.channel_mut(channel) {
            channel.push(TranscriptLine::System(text.into()));
        }
    }

    /// Says that a save is over, and how it went (`notice`), where its `mode`
    /// puts it: a save someone asked for is answered in the channel they are
    /// in, as it always was; an automatic one in the channel its file arrived
    /// in -- and is no longer in flight, so the same file can be saved again
    /// (`auto_saving`).
    pub(super) fn finish_save(
        &mut self,
        channel: &str,
        hash: Hash,
        mode: SaveMode,
        notice: String,
    ) {
        match mode {
            SaveMode::Manual => self.push_system(notice),
            SaveMode::Automatic { .. } => {
                self.auto_saving.remove(&(channel.to_string(), hash));
                self.push_system_to(channel, notice);
            }
        }
    }

    /// Says that a save didn't happen -- the download failed
    /// (`NetEvent::FileSaveFailed`) or there was nowhere to put the file
    /// (main.rs's `start_save`) -- `error` being the reason. A save someone
    /// asked for says so exactly as it always did; an automatic one says
    /// which file and how to fetch it by hand, in the channel it arrived in.
    pub fn save_failed(
        &mut self,
        channel: &str,
        hash: Hash,
        filename: &str,
        error: &str,
        mode: SaveMode,
    ) {
        let notice = match mode {
            SaveMode::Manual => format!("failed to save {filename}: {error}"),
            SaveMode::Automatic { .. } => format!(
                "couldn't auto-save {filename}: {error} -- {} to download it yourself",
                save_hint(hash)
            ),
        };
        self.finish_save(channel, hash, mode, notice);
    }
}
