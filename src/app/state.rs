use super::*;

impl AppState {
    /// Seeds `channel`'s transcript from persisted history loaded by the
    /// caller (see storage.rs). A no-op if `channel` isn't currently
    /// joined. Intended to be called once per joined channel right after
    /// `AppState::new`, before the event loop starts.
    pub fn load_history(&mut self, channel: &str, messages: Vec<ChatMessage>) {
        for message in &messages {
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
        }
        if let Some(channel) = self.channel_mut(channel) {
            channel.seed_history(messages);
        }
    }

    /// Seeds local pet names persisted by the caller (see contacts.rs).
    /// Intended to be called once, right after `AppState::new`, before
    /// the event loop starts -- mirrors `load_history`.
    pub fn load_contacts(&mut self, petnames: HashMap<[u8; 32], String>) {
        self.petnames = petnames;
    }
    /// Seeds remembered peer observations loaded by the caller. Live gossip
    /// events still decide online state; this data only keeps peers visible
    /// after they leave and across restarts.
    pub fn load_last_seen(&mut self, last_seen: HashMap<String, HashMap<[u8; 32], u64>>) {
        for channel in &mut self.channels {
            if let Some(peers) = last_seen.get(&channel.name) {
                channel.last_seen = peers.clone();
            }
        }
    }

    /// Records a peer observation in the in-memory state. The runtime owns
    /// persistence and calls this alongside `LastSeen::record`.
    pub fn record_peer_seen(&mut self, channel: &str, id: [u8; 32], seen_unix_ms: u64) {
        if let Some(channel) = self.channel_mut(channel) {
            channel.last_seen.insert(id, seen_unix_ms);
        }
    }

    /// Seeds our own canonical user identity (see `self_user_id`), loaded
    /// by the caller from `net::Net::our_user_id`. Intended to be called
    /// once, right after `AppState::new`, before the event loop starts --
    /// mirrors `load_contacts`.
    pub fn load_self_user_id(&mut self, user_id: [u8; 32]) {
        self.self_user_id = user_id;
    }

    /// Seeds persisted local preferences loaded by the caller (see
    /// settings.rs). Intended to be called once, right after
    /// `AppState::new`, before the event loop starts -- mirrors
    /// `load_contacts`.
    pub fn load_settings(
        &mut self,
        bell_enabled: bool,
        mouse_enabled: bool,
        nickname: Option<String>,
    ) {
        self.bell_enabled = bell_enabled;
        self.mouse_enabled = mouse_enabled;
        self.local_nickname = nickname;
    }

    /// Seeds known peer <-> DM channel associations persisted by the
    /// caller (see `crate::dm_registry`). Intended to be called once,
    /// right after `AppState::new`, before the event loop starts --
    /// mirrors `load_contacts`.
    pub fn load_dm_registry(&mut self, dm_channels: HashMap<[u8; 32], String>) {
        self.dm_channels = dm_channels;
    }

    /// Seeds forgotten device ids persisted by the caller (see
    /// `crate::forgotten_devices`). Intended to be called once, right
    /// after `AppState::new`, before the event loop starts -- mirrors
    /// `load_contacts`.
    pub fn load_forgotten_devices(&mut self, forgotten: HashSet<[u8; 32]>) {
        self.forgotten = forgotten;
    }

    /// Seeds the local channel display names persisted by the caller (see
    /// `crate::channel_renames`). Intended to be called once, right after
    /// `AppState::new`, before the event loop starts -- mirrors
    /// `load_contacts`.
    pub fn load_renames(&mut self, renames: HashMap<String, String>) {
        self.renames = renames;
    }

    /// Seeds the folder the file picker opens in (see
    /// `picker::start_folder`, which resolves what was remembered across
    /// restarts). Intended to be called once, right after `AppState::new`,
    /// before the event loop starts -- mirrors `load_contacts`.
    pub fn load_browse_dir(&mut self, dir: PathBuf) {
        self.browse_dir = dir;
    }

    /// Seeds the files root (see `files::files_root`, which resolves what
    /// was remembered across restarts) -- and replaces it once a `/folder
    /// <path>` has taken effect (`InputAction::SetFilesRoot`). Intended to be
    /// called once, right after `AppState::new`, before the event loop
    /// starts -- mirrors `load_browse_dir`.
    pub fn load_files_root(&mut self, root: PathBuf) {
        self.files_root = root;
    }

    /// Associates one direct channel with its persisted local folder name.
    /// The caller owns persistence; app state only resolves paths from the
    /// already validated mapping.
    pub fn load_direct_folder(&mut self, channel: String, folder: String) {
        self.direct_folders.insert(channel, folder);
    }

    pub fn load_shared_files(&mut self, files: HashMap<String, Vec<SharedFile>>) {
        self.shared_files = files;
    }

    pub fn record_shared_file(&mut self, channel: &str, shared: SharedFile) {
        let files = self.shared_files.entry(channel.to_string()).or_default();
        if files
            .iter()
            .any(|known| known.message_id == shared.message_id)
        {
            return;
        }
        files.push(shared);
        files.sort_by_key(|shared| std::cmp::Reverse((shared.ts_unix_ms, shared.message_id)));
    }

    /// The physical folder for `channel`. Direct mappings are local-only;
    /// every other channel retains its original stored-name folder.
    pub fn folder_dir(&self, channel: &str) -> PathBuf {
        let folder = self
            .direct_folders
            .get(channel)
            .map_or(channel, String::as_str);
        channel_dir(&self.files_root, folder)
    }

    /// Seeds the size limit (see `files::size_limit`, which resolves what
    /// was remembered across restarts). Intended to be called once, right
    /// after `AppState::new`, before the event loop starts -- mirrors
    /// `load_files_root`.
    pub fn load_size_limit(&mut self, limit: SizeLimit) {
        self.size_limit = limit;
    }

    pub fn load_autosave_limit(&mut self, limit: AutoSaveLimit) {
        self.autosave_limit = limit;
    }
    pub fn load_autosave_limits(&mut self, limits: HashMap<String, AutoSaveLimit>) {
        self.autosave_limits = limits;
    }
    pub(super) fn autosave_limit_for(&self, channel: &str) -> AutoSaveLimit {
        self.autosave_limits
            .get(channel)
            .copied()
            .unwrap_or(self.autosave_limit)
    }

    /// Seeds the channels with auto-save on (see `autosave::AutoSave`, which
    /// holds what was remembered across restarts). Intended to be called
    /// once, right after `AppState::new`, before the event loop starts --
    /// mirrors `load_renames`.
    pub fn load_autosave(&mut self, channels: HashSet<String>) {
        self.autosave = channels;
    }

    /// Seeds the enabled one-way outboxes persisted by the caller.
    pub fn load_outbox(&mut self, channels: HashSet<String>) {
        self.outbox = channels;
    }

    pub fn load_sync(&mut self, configs: HashMap<String, SyncConfig>) {
        self.sync = configs;
    }

    pub fn sync_config(&self, channel: &str) -> SyncConfig {
        self.sync.get(channel).cloned().unwrap_or_default()
    }
    pub fn sync_receives(&self, channel: &str) -> bool {
        self.sync_config(channel).receives()
    }
    pub fn sync_paused(&self, channel: &str) -> bool {
        self.sync_config(channel).is_paused()
    }
    pub fn sync_allows_path(&self, channel: &str, path: &str) -> bool {
        self.sync_config(channel).allows_path(path)
    }

    pub fn sync_status(&self, channel: &str) -> Option<&SyncStatus> {
        self.sync_status.get(channel)
    }
    pub fn sync_is_pending(&self, channel: &str, path: &str) -> bool {
        self.sync_status
            .get(channel)
            .is_some_and(|status| status.pending.contains(path))
    }

    pub fn sync_pending(&mut self, channel: &str, path: String) {
        self.sync_status
            .entry(channel.to_string())
            .or_default()
            .pending(path);
    }

    pub fn sync_succeeded(&mut self, channel: &str, path: &str, completed_unix_ms: u64) {
        self.sync_status
            .entry(channel.to_string())
            .or_default()
            .succeeded(path, completed_unix_ms);
    }

    pub fn sync_failed(&mut self, channel: &str, path: String, error: String) {
        self.sync_status
            .entry(channel.to_string())
            .or_default()
            .failed(path, error);
    }

    pub fn sync_conflict(&mut self, channel: &str, conflict: SyncConflict) {
        self.sync_status
            .entry(channel.to_string())
            .or_default()
            .conflict(conflict);
    }
    pub fn sync_conflicts(&self, channel: &str) -> Vec<SyncConflict> {
        self.sync_status(channel)
            .map(|status| status.conflicts().to_vec())
            .unwrap_or_default()
    }
    pub fn sync_conflict_resolved(&mut self, channel: &str, conflict: &SyncConflict) {
        if let Some(status) = self.sync_status.get_mut(channel) {
            status.conflicts.retain(|known| known != conflict);
        }
    }

    pub fn sync_marker(&self, channel: &str) -> Option<char> {
        let config = self.sync_config(channel);
        if config.policy == SyncPolicy::Off {
            None
        } else if config.is_paused() {
            Some('P')
        } else if self
            .sync_status(channel)
            .is_some_and(|status| status.error_count() > 0)
        {
            Some('!')
        } else if self
            .sync_status(channel)
            .is_some_and(|status| status.conflict_count() > 0)
        {
            Some('C')
        } else if self
            .sync_status(channel)
            .is_some_and(|status| status.pending_count() > 0)
        {
            Some('~')
        } else {
            Some('S')
        }
    }

    /// Applies a successfully persisted outbox change from the runtime.
    pub fn set_outbox(&mut self, channel: String, on: bool) {
        if on {
            self.outbox.insert(channel);
        } else {
            self.outbox.remove(&channel);
        }
    }

    /// How big a file may be to share right now (`/maxsize`).
    pub fn size_limit(&self) -> SizeLimit {
        self.size_limit
    }

    /// Merges a batch of possibly-historical messages fetched via backfill
    /// (see backfill.rs, `NetEvent::HistoryFetched`) into `channel`'s
    /// transcript. A no-op if `channel` isn't currently joined.
    ///
    /// Dedupes by id using the same `remember_seen` state as live messages.
    /// Unlike live messages, which are simply appended, the channel's
    /// `Chat` lines are then rebuilt sorted by `(ts_unix_ms, id)`, since a
    /// backfilled batch can be older than what's already displayed and
    /// mustn't be shown as the newest lines. `System` lines (e.g. earlier
    /// invite output or errors) are left as-is, trailing after the
    /// resorted chat history -- an accepted simplification, since backfill
    /// is expected to land early in a session before much else happens.
    ///
    /// Returns the newly-accepted messages, in canonical order, so the
    /// caller can persist just those to storage.rs without duplicating
    /// anything already on disk.
    pub fn merge_history(&mut self, channel: &str, messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
        let channel_name = channel.to_string();
        let Some(channel) = self.channel_mut(channel) else {
            return Vec::new();
        };
        let mut newly_accepted: Vec<ChatMessage> = messages
            .into_iter()
            .filter(|message| channel.remember_seen(message.id))
            .collect();
        if newly_accepted.is_empty() {
            return newly_accepted;
        }

        let mut chats = Vec::with_capacity(channel.messages.len() + newly_accepted.len());
        let mut systems = Vec::new();
        for line in channel.messages.drain(..) {
            match line {
                TranscriptLine::Chat(message) => chats.push(message),
                TranscriptLine::System(text) => systems.push(text),
            }
        }
        chats.extend(newly_accepted.iter().cloned());
        chats.sort_by_key(|m| (m.ts_unix_ms, m.id));

        for message in chats {
            channel.messages.push_back(TranscriptLine::Chat(message));
        }
        for text in systems {
            channel.messages.push_back(TranscriptLine::System(text));
        }
        while channel.messages.len() > MAX_SCROLLBACK {
            channel.messages.pop_front();
        }

        newly_accepted.sort_by_key(|m| (m.ts_unix_ms, m.id));
        for message in &newly_accepted {
            if let Some(file) = &message.attachment {
                self.record_shared_file(
                    &channel_name,
                    SharedFile {
                        message_id: message.id,
                        file: file.clone(),
                        sender: message.sender,
                        ts_unix_ms: message.ts_unix_ms,
                    },
                );
            }
        }
        newly_accepted
    }

    /// Merges a finished background on-disk scan (`search::run_on_disk_scan`,
    /// kicked off via `InputAction::Search`) into `outcome.channel`'s active
    /// search -- a no-op if that channel isn't currently joined, or if its
    /// active search has since changed (comparing `outcome.term` against
    /// the channel's currently-active one rejects a stale result from a
    /// `/search` that's been superseded or cleared, without needing a
    /// separate request-id).
    ///
    /// Unions rather than replaces the existing snapshot, deduped by id and
    /// re-sorted by `(ts_unix_ms, id)` (mirroring `merge_history`'s
    /// convention above): the on-disk log is a superset of the loaded
    /// scrollback in the overwhelmingly common case, but treating a read
    /// failure (reported as an empty `SearchOutcome`, see
    /// `search::run_on_disk_scan`) as "replace with nothing" would
    /// otherwise erase the in-memory matches already shown.
    pub fn apply_search_outcome(&mut self, outcome: SearchOutcome) {
        let Some(channel) = self.channel_mut(&outcome.channel) else {
            return;
        };
        let Some(search) = &mut channel.search else {
            return;
        };
        if search.term != outcome.term {
            return;
        }
        let mut seen: HashSet<u64> = search.messages.iter().map(|message| message.id).collect();
        for message in outcome.messages {
            if seen.insert(message.id) {
                search.messages.push(message);
            }
        }
        search
            .messages
            .sort_by_key(|message| (message.ts_unix_ms, message.id));
        search.pending = false;
    }

    /// Hands a finished folder read (`picker::read_folder`, kicked off via
    /// `InputAction::ReadFolder`) to the open file picker. A no-op if the
    /// picker has since closed; whether the answer is the one it was
    /// waiting for is the picker's call (`FilePicker::apply`).
    pub fn apply_folder_listing(&mut self, listing: FolderListing) {
        if let Some(picker) = &mut self.picking_file {
            picker.apply(listing);
        }
    }

    pub fn active(&self) -> &Channel {
        &self.channels[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Channel {
        &mut self.channels[self.active]
    }

    pub(super) fn channel_mut(&mut self, name: &str) -> Option<&mut Channel> {
        self.channels.iter_mut().find(|c| c.name == name)
    }

    /// Clears the newly-active channel's unread state, retaining its oldest
    /// unread message as the renderer's viewport anchor, and clears any
    /// in-progress reply pick or armed reply -- both are only meaningful
    /// against the channel they were made in. The shared tail end of every
    /// place that changes `active`: `switch_channel` (`Tab`/`Shift+Tab`),
    /// `activate_existing_channel`, and `activate_or_create_channel`.
    pub(super) fn finish_activating_channel(&mut self) {
        let channel = self.active_mut();
        channel.unread_anchor = channel.first_unread.take();
        channel.unread_count = 0;
        self.picking_reply = None;
        self.picking_thread = None;
        self.replying_to = None;
    }

    /// Switches to `name` if it's currently joined, via
    /// `finish_activating_channel`. Returns whether `name` was found --
    /// `/msg`'s reuse path (`run_msg`) uses this to fall back to creating
    /// a brand-new DM channel when the one `crate::dm_registry` remembers
    /// isn't (or is no longer) actually joined.
    pub(super) fn activate_existing_channel(&mut self, name: &str) -> bool {
        let Some(index) = self.channels.iter().position(|c| c.name == name) else {
            return false;
        };
        self.active = index;
        self.finish_activating_channel();
        true
    }

    /// Switches to `name`, creating a new tab for it first if it isn't
    /// already joined -- shared by `NetEvent::Joined` and
    /// `NetEvent::DmJoined`, both of which report a channel (new or
    /// already-known) finishing its join.
    pub(super) fn activate_or_create_channel(&mut self, name: String) {
        if !self.activate_existing_channel(&name) {
            self.channels.push(Channel::new(name));
            self.active = self.channels.len() - 1;
            self.finish_activating_channel();
        }
    }

    /// Adds `name` as a new, joined-but-inactive tab -- for a channel
    /// learned via the private device-sync channel (see
    /// `NetEvent::ChannelSyncJoined`), which should appear once actually
    /// joined without yanking focus away from whatever the user is
    /// currently looking at, unlike `activate_or_create_channel`'s
    /// deliberate tab-switch for a user-initiated `/join`. A no-op if
    /// `name` is already a tab (e.g. this device already knew about it
    /// some other way).
    pub(super) fn add_background_channel(&mut self, name: String) {
        if !self.channels.iter().any(|c| c.name == name) {
            self.channels.push(Channel::new(name));
        }
    }

    /// Records `peer` <-> `channel` in `dm_channels` -- called by main.rs
    /// once a `/msg`-initiated `InputAction::StartDm` actually finishes
    /// joining (a plain `NetEvent::Joined`, correlated back to `peer` via
    /// main.rs's own `pending_dms` map, since a freshly created channel's
    /// `Joined` event has no ticket to carry `peer` through the way
    /// `NetEvent::DmJoined` does for the recipient side -- see that
    /// variant's arm in `handle_net_event`, which updates `dm_channels`
    /// itself instead of needing this).
    pub fn record_dm_channel(&mut self, peer: [u8; 32], channel: String) {
        self.dm_channels.insert(peer, channel);
    }

    /// Classifies `channel` (by its own `name`, the identity key) as a
    /// group channel, a `/msg` DM, or a `/dm` point-to-point chat, and --
    /// for the latter two -- says which peer it's with. The one place that
    /// answers that question: `dm_channels` and `direct_channels` are both
    /// keyed peer -> channel name, so classifying by name is a reverse
    /// lookup. A name present in neither is a group channel, including one
    /// that merely *looks* like a DM (e.g. a user-created `#dm-alice`).
    pub fn channel_kind(&self, channel: &str) -> ChannelKind {
        if let Some(peer) = peer_for_channel(&self.direct_channels, channel) {
            ChannelKind::Direct { peer }
        } else if let Some(peer) = peer_for_channel(&self.dm_channels, channel) {
            ChannelKind::GossipDm { peer }
        } else {
            ChannelKind::Group
        }
    }

    /// The name to *show* a user for `channel`, as opposed to its `name`,
    /// which is its identity (storage file, registry key, gossip map
    /// entry) and must never change. A group channel shows its stored
    /// name, or the local one `/rename` gave it (`renames`). A DM or
    /// point-to-point chat shows whoever it's with, resolved
    /// through `display_name` on every call, so it follows `/alias` and
    /// `/nick` immediately instead of staying frozen at whatever was known
    /// when the channel was created. That frozen name was worse than stale
    /// for the *recipient* of a `/msg` invite: the initiator picks it
    /// (`unique_dm_channel_name`) from their label for the recipient, so on
    /// the recipient's side it described themselves rather than the person
    /// they were talking to.
    ///
    /// Just the name, with no kind marker -- a `/msg` DM and a `/dm` chat
    /// with the same person share one. See `channel_heading` for the form
    /// that says which kind it is.
    pub fn channel_label(&self, channel: &str) -> String {
        match self.channel_kind(channel) {
            ChannelKind::Group => self
                .renames
                .get(channel)
                .cloned()
                .unwrap_or_else(|| channel.to_string()),
            ChannelKind::GossipDm { peer } | ChannelKind::Direct { peer } => {
                self.display_name(&peer)
            }
        }
    }

    /// `channel_label` behind its kind's glyph (`#general`, `@Alice`, a
    /// bolt before `Bob`) -- how a channel is named in pane titles and
    /// system notices, so its kind reads the same everywhere as it does in
    /// the navigation panel, and so two channels that share a label stay
    /// distinguishable.
    pub fn channel_heading(&self, channel: &str) -> String {
        format!(
            "{}{}",
            self.channel_kind(channel).glyph(),
            self.channel_label(channel)
        )
    }

    /// How the navigation panel groups the joined channels: group channels
    /// under one section, DMs and point-to-point chats under the other,
    /// each kept in join order. Every channel lands in exactly one.
    pub fn nav_sections(&self) -> NavSections {
        let mut sections = NavSections {
            channels: Vec::new(),
            people: Vec::new(),
        };
        for (index, channel) in self.channels.iter().enumerate() {
            match self.channel_kind(&channel.name) {
                ChannelKind::Group => sections.channels.push(index),
                ChannelKind::GossipDm { .. } | ChannelKind::Direct { .. } => {
                    sections.people.push(index);
                }
            }
        }
        sections
    }

    /// Every channel index in the order the navigation panel draws them,
    /// top to bottom -- the order `Tab`/`Shift+Tab` step through
    /// (`switch_channel`).
    pub(super) fn nav_order(&self) -> Vec<usize> {
        let NavSections {
            mut channels,
            people,
        } = self.nav_sections();
        channels.extend(people);
        channels
    }

    /// The peer a point-to-point channel belongs to, if `channel` is one
    /// -- see `channel_kind`. Used by main.rs to decide whether a send (or
    /// a `/leave`) should go through `net::Net::send_direct`/
    /// `close_direct` instead of the gossip-based `send`/`leave`.
    pub fn direct_peer_for(&self, channel: &str) -> Option<[u8; 32]> {
        match self.channel_kind(channel) {
            ChannelKind::Direct { peer } => Some(peer),
            ChannelKind::Group | ChannelKind::GossipDm { .. } => None,
        }
    }

    /// Switches to (creating if necessary) `peer`'s point-to-point
    /// channel, named `channel` (see `direct::channel_name`), and records
    /// the association in `direct_channels`. Returns whether this is a
    /// brand-new tab -- the caller (main.rs) only needs to load persisted
    /// history (`storage::MessageStore`) in that case, mirroring how
    /// startup only loads each gossip channel's history once. Backs the
    /// user-initiated path (`run_dm`'s `InputAction::OpenDirect`): the
    /// local user explicitly asked to go here, so -- like
    /// `activate_or_create_channel` -- this switches focus.
    pub fn activate_direct_channel(&mut self, channel: String, peer: [u8; 32]) -> bool {
        let is_new = !self.channels.iter().any(|c| c.name == channel);
        self.activate_or_create_channel(channel.clone());
        self.direct_channels.insert(peer, channel);
        is_new
    }

    /// Adds `peer`'s point-to-point channel as a background tab without
    /// switching focus, and records the association in `direct_channels`
    /// -- the recipient-side counterpart to `activate_direct_channel`,
    /// for a peer who reached us first with no prior contact on our side
    /// (`net::NetEvent::DirectJoined`). Mirrors `add_background_channel`'s
    /// "don't yank focus" reasoning: the local user never asked for this
    /// conversation to appear. Returns whether this is a brand-new tab,
    /// like `activate_direct_channel`.
    pub fn background_direct_channel(&mut self, channel: String, peer: [u8; 32]) -> bool {
        let is_new = !self.channels.iter().any(|c| c.name == channel);
        self.add_background_channel(channel.clone());
        self.direct_channels.insert(peer, channel);
        is_new
    }
}
