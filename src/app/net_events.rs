//! Application-state updates driven by network events.

use super::*;

impl AppState {
    /// `Received`.
    pub fn handle_net_event(&mut self, event: NetEvent) -> Option<(String, ChatMessage)> {
        match event {
            NetEvent::PeerJoined(channel_name, id) => {
                if let Some(channel) = self.channel_mut(&channel_name)
                    && !channel.peers.contains(&id)
                {
                    channel.peers.push(id);
                }
                None
            }
            NetEvent::PeerLeft(channel_name, id) => {
                if let Some(channel) = self.channel_mut(&channel_name) {
                    channel.peers.retain(|peer| *peer != id);
                }
                None
            }
            NetEvent::Received(channel_name, message) => {
                let is_active = self.active().name == channel_name;
                let channel = self.channel_mut(&channel_name)?;
                if !channel.remember_seen(message.id) {
                    return None;
                }
                channel.push(TranscriptLine::Chat(message.clone()));
                if !is_active {
                    channel.has_unread = true;
                }
                Some((channel_name, message))
            }
            NetEvent::Lagged(channel_name) => {
                if let Some(channel) = self.channel_mut(&channel_name) {
                    channel.push(TranscriptLine::System(
                        "some messages may have been missed (lagged)".to_string(),
                    ));
                }
                None
            }
            NetEvent::Joined(name) => {
                self.activate_or_create_channel(name);
                None
            }
            // The recipient side of a `/msg`-initiated DM (see
            // `ticket::ChannelTicket::dm` and `net::NetEvent::DmJoined`):
            // activate it exactly like a plain `Joined` would, and also
            // record the ticket's sharer as our DM partner for this
            // channel, so a later `/msg` to them reuses it instead of
            // creating a second, separate room. The initiator's own side
            // never sees this variant -- see main.rs's `pending_dms` for
            // how it records the same association from a plain `Joined`.
            NetEvent::DmJoined { channel, peer } => {
                self.activate_or_create_channel(channel.clone());
                self.dm_channels.insert(peer, channel);
                None
            }
            NetEvent::JoinFailed(name, error) => {
                self.push_system(format!("failed to join #{name}: {error}"));
                None
            }
            // A channel learned via the private device-sync channel
            // finished subscribing -- see `net::NetEvent::ChannelSyncJoined`
            // and `net::Net::join_known`. Adds a background tab rather
            // than switching to it, unlike a plain `Joined`: the user
            // never asked to go there, a sibling device just did.
            NetEvent::ChannelSyncJoined(name) => {
                self.add_background_channel(name);
                None
            }
            // Raw `ChannelSync` announces are acted on by `net::Net::
            // sync_channel` (called from main.rs), which reports back as
            // `ChannelSyncJoined` above once (and if) the join actually
            // succeeds -- nothing for app.rs to do with the raw announce
            // itself.
            NetEvent::ChannelSync(..) => None,
            // A peer's (re-)announced broadcast nickname -- see
            // features.md's "Broadcast nicknames". Simply overwrite
            // whatever we last had for `sender`'s resolved identity, so
            // `display_name` always reflects the most recently seen one.
            // Keyed by `canonical_id` (not the raw sender) so a nickname
            // broadcast by any one of a person's devices is seen as that
            // person's nickname everywhere -- see `NetEvent::Device` below
            // for how an entry already recorded under a bare device id
            // gets migrated once that device's certificate arrives.
            NetEvent::Identity(_channel, identity) => {
                let id = self.canonical_id(&identity.sender);
                self.nicknames.insert(id, identity.nickname);
                None
            }
            // A peer's (re-)announced build version -- see features.md's
            // version-notification idea. Always recorded (for `/who` and
            // `newest_known_update`), regardless of whether it's newer
            // than ours; a system notice only fires the first time this
            // peer's recorded version actually becomes newer than ours,
            // so a plain reconnect (the same version re-announced) doesn't
            // repeat it. Keyed by `canonical_id`, like `nicknames` above.
            NetEvent::Version(_channel, announce) => {
                let id = self.canonical_id(&announce.sender);
                let info = (announce.version.clone(), announce.git_hash.clone());
                let previous = self.peer_versions.insert(id, info.clone());
                if previous.as_ref() != Some(&info) && version::is_newer(&announce.version) {
                    let who = self.display_name(&announce.sender);
                    self.push_system(format!(
                        "{who} is running v{} (you're on v{}) -- consider updating",
                        announce.version,
                        version::VERSION,
                    ));
                }
                None
            }
            // A peer's device certificate -- see `crate::message::DeviceCert`.
            // Already verified by `net::forward_events` before this ever
            // arrives, so it's simply recorded. The first time a device id
            // newly resolves to a user id, migrate anything already
            // recorded under that bare device id (a nickname/version seen
            // before its cert, since gossip delivery has no ordering
            // guarantee) onto the now-known canonical id, so it isn't
            // orphaned under a key `display_name`/`who_line` no longer
            // look up.
            NetEvent::Device(_channel, cert) => {
                let previous = self.device_to_user.insert(cert.device_id, cert.user_id);
                if previous != Some(cert.user_id) {
                    if let Some(nickname) = self.nicknames.remove(&cert.device_id) {
                        self.nicknames.insert(cert.user_id, nickname);
                    }
                    if let Some(info) = self.peer_versions.remove(&cert.device_id) {
                        self.peer_versions.insert(cert.user_id, info);
                    }
                }
                None
            }
            // A saved file finished downloading -- see
            // `net::Net::save_file`. main.rs has already recorded where it
            // went (`saved_files`); what is left is the notice -- said where
            // the save's `mode` says (`finish_save`) -- and, if the
            // files overlay is open on the channel it was saved for, the
            // check mark -- put there now rather than at the overlay's next
            // opening.
            NetEvent::FileSaved {
                channel,
                hash,
                filename,
                path,
                mode,
            } => {
                if let Some(overlay) = &mut self.viewing_files
                    && overlay.channel == channel
                {
                    overlay.saved.insert(hash, path.clone());
                }
                let notice = match mode {
                    SaveMode::Manual => format!("saved {filename} to {}", path.display()),
                    SaveMode::Automatic { .. } => {
                        format!("auto-saved {filename} to {}", path.display())
                    }
                };
                self.finish_save(&channel, hash, mode, notice);
                None
            }
            // Handled in main.rs before reaching here: `Announce` triggers
            // `Net::sync_history`, `HistoryFetched`'s payload goes through
            // `merge_history` above, `PeerAddressLearned` is persisted to
            // the channel registry (see channel_registry.rs), `FileReady`
            // broadcasts and persists a finished `/send` (or `/paste`)
            // before placing it in the transcript via `record_sent_message`,
            // `FileSendFailed`/`FileSaveFailed` are also logged there via
            // `tracing::warn!` (unlike the other system notices here, a
            // failed file transfer is worth a trace in leyline.log too),
            // `ClipboardFiles`/`ClipboardImage`/`ClipboardText`/
            // `PasteFailed` (see `net::Net::paste`) are turned into a
            // staged attachment (`stage_attachment`), a plain `Send`, or
            // a logged error,
            // and `DirectJoined` is turned into a background tab via
            // `background_direct_channel` (main.rs needs to conditionally
            // load persisted history alongside it, which this method's
            // `Option<(String, ChatMessage)>` return shape has no room to
            // express). None of these are transcript or presence state
            // handled here directly.
            NetEvent::Announce(..)
            | NetEvent::HistoryFetched(..)
            | NetEvent::PeerAddressLearned(..)
            | NetEvent::FileReady(..)
            | NetEvent::FileSendFailed { .. }
            | NetEvent::FileSaveFailed { .. }
            | NetEvent::ClipboardFiles(..)
            | NetEvent::ClipboardImage(..)
            | NetEvent::ClipboardText(..)
            | NetEvent::PasteFailed(..)
            | NetEvent::RemoteIndex(..)
            | NetEvent::DirectJoined { .. } => None,
        }
    }
}
