//! Owns all UI state and the pure logic that mutates it.
//!
//! Per concept.md's "one owner of UI state" principle: nothing here does
//! network or file I/O. `handle_key` is a pure function of the current
//! state and a keypress -- when the user sends a message or runs a `/join`
//! or `/invite` command, it returns an `InputAction` describing what the
//! caller should do, rather than reaching for I/O itself.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::message::ChatMessage;
use crate::net::NetEvent;
use crate::search::{self, SearchOutcome, SearchResults};

/// Cap on in-memory scrollback per channel, per concept.md's guidance on
/// bounding render/memory cost in a long-lived session.
const MAX_SCROLLBACK: usize = 500;

/// Cap on how many recently-seen message ids we remember per channel.
/// Gossip is best-effort and can redeliver, so incoming messages are
/// deduped by id -- see concept.md's "Message wire format" section.
const MAX_SEEN_IDS: usize = 256;

/// A line in a channel's transcript: either a real chat message or a
/// local-only notice (invite output, command errors). System lines never
/// travel over the network -- see message.rs for the wire format. Named
/// `TranscriptLine` (not `Line`) to avoid colliding with `ratatui::text::Line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptLine {
    Chat(ChatMessage),
    System(String),
}

/// One joined channel's transcript, presence, and per-channel scroll state.
pub struct Channel {
    pub name: String,
    pub messages: VecDeque<TranscriptLine>,
    pub peers: Vec<[u8; 32]>,
    pub scroll: usize,
    /// Whether this channel has unseen activity, shown in the tab bar --
    /// cleared when it becomes the active channel.
    pub has_unread: bool,
    /// Recently-seen message ids, oldest first, for incoming-message dedupe.
    seen_ids: VecDeque<u64>,
    /// Active `/search <term>` results, if any -- see `AppState::run_search`
    /// and `ui::render_messages`. Transient UI state, like `scroll`: never
    /// persisted, and not kept live as new messages arrive (re-run
    /// `/search <term>` to refresh).
    pub search: Option<SearchResults>,
}

impl Channel {
    fn new(name: String) -> Self {
        Self {
            name,
            messages: VecDeque::new(),
            peers: Vec::new(),
            scroll: 0,
            has_unread: false,
            seen_ids: VecDeque::new(),
            search: None,
        }
    }

    /// Records `id` as seen, evicting the oldest entry once over capacity.
    /// Returns `true` if `id` had not already been seen (i.e. it should be
    /// displayed).
    fn remember_seen(&mut self, id: u64) -> bool {
        if self.seen_ids.contains(&id) {
            return false;
        }
        self.seen_ids.push_back(id);
        if self.seen_ids.len() > MAX_SEEN_IDS {
            self.seen_ids.pop_front();
        }
        true
    }

    fn push(&mut self, line: TranscriptLine) {
        self.messages.push_back(line);
        while self.messages.len() > MAX_SCROLLBACK {
            self.messages.pop_front();
        }
    }

    /// Seeds this channel's transcript and dedupe state from persisted
    /// history (see storage.rs), oldest message first. Reuses the same
    /// dedupe/cap logic as live messages so loaded history behaves
    /// identically to messages received during this session.
    fn seed_history(&mut self, messages: Vec<ChatMessage>) {
        for message in messages {
            if self.remember_seen(message.id) {
                self.push(TranscriptLine::Chat(message));
            }
        }
    }

    /// Scans this channel's loaded transcript for `Chat` lines whose text
    /// matches `term_lower` (already lowercased), oldest first -- the
    /// synchronous half of `/search`. See `search::run_on_disk_scan` for
    /// the background half covering history older than what's loaded here.
    fn search_loaded(&self, term_lower: &str) -> Vec<ChatMessage> {
        self.messages
            .iter()
            .filter_map(|line| match line {
                TranscriptLine::Chat(message) if search::matches(&message.text, term_lower) => {
                    Some(message.clone())
                }
                _ => None,
            })
            .collect()
    }
}

/// An action for the caller (`main.rs`) to perform in response to a
/// keypress, since `AppState` itself never does network I/O.
pub enum InputAction {
    /// Broadcast a composed message to the named channel.
    Send(String, ChatMessage),
    /// Join (or create) a channel from a bare name or an invite ticket
    /// string, exactly as the user typed it after `/join `.
    Join(String),
    /// Build an invite ticket for the named channel and report it back
    /// (via `AppState::push_system`) once built.
    Invite(String),
    /// Persist a local pet name for a specific endpoint id, resolved from
    /// a typed hex-prefix by `/alias` (see `run_command` and
    /// `crate::contacts`).
    Alias([u8; 32], String),
    /// Broadcast a chosen display nickname to every joined channel, as
    /// typed after `/nick` (see `run_command` and
    /// `crate::net::Net::set_nickname`).
    Nick(String),
    /// Leave a previously joined channel -- already resolved to a
    /// concrete, currently-joined name by `run_leave` (either explicitly
    /// typed after `/leave`, or the active channel if omitted). The
    /// caller is responsible for actually tearing down its gossip
    /// subscription and persisted state (see `crate::net::Net::leave`,
    /// `crate::channel_registry`, `crate::storage`, `crate::backfill`)
    /// and then calling `AppState::remove_channel`.
    Leave(String),
    /// Kick off a background scan of `channel`'s full on-disk log for
    /// `term` (already applied synchronously to the loaded scrollback by
    /// `run_search`) -- see `search::run_on_disk_scan`,
    /// `AppState::apply_search_outcome`, and main.rs.
    Search { channel: String, term: String },
}

/// All state needed to render the TUI and respond to input.
pub struct AppState {
    /// Our own identity, so we can tell our messages apart from peers'.
    pub self_id: [u8; 32],
    pub channels: Vec<Channel>,
    pub active: usize,
    pub input: String,
    pub cursor: usize,
    pub should_quit: bool,
    /// Whether the sidebar's command hints panel is shown -- toggled via
    /// `/hints` (`run_hints`). Visible by default so the panel actually
    /// teaches new users; in-memory only, like `scroll`/`input`, not
    /// persisted across restarts.
    pub show_hints: bool,
    /// Local pet names assigned via `/alias`, keyed by endpoint id --
    /// checked by `display_name` before falling back to a hex prefix.
    /// Seeded once at startup from `crate::contacts::Contacts` (see
    /// `load_contacts`); never sent over the wire.
    petnames: HashMap<[u8; 32], String>,
    /// Last-seen broadcast nickname per sender, from `/nick` (see
    /// `NetEvent::Identity`) -- checked by `display_name` after `petnames`
    /// but before the hex-prefix fallback. In-memory only, unlike
    /// `petnames`: a broadcast nickname isn't ours to persist, and its
    /// owner re-announces it on every reconnect anyway.
    nicknames: HashMap<[u8; 32], String>,
}

impl AppState {
    /// Builds the initial state for a session with the given identity and
    /// already-joined channel names (in join order). `active_channel`
    /// selects which one starts active -- e.g. so `--join
    /// <ticket-for-project-x>` lands you in `project-x`, not `general` --
    /// falling back to the first channel if `active_channel` isn't among
    /// `channel_names`.
    ///
    /// # Panics
    /// Panics if `channel_names` is empty. `AppState` always has at least
    /// one channel, so `active` is always a valid index.
    pub fn new(self_id: [u8; 32], channel_names: Vec<String>, active_channel: &str) -> Self {
        assert!(
            !channel_names.is_empty(),
            "AppState must start with at least one channel"
        );
        let active = channel_names
            .iter()
            .position(|name| name == active_channel)
            .unwrap_or(0);
        let channels: Vec<Channel> = channel_names.into_iter().map(Channel::new).collect();
        Self {
            self_id,
            channels,
            active,
            input: String::new(),
            cursor: 0,
            should_quit: false,
            show_hints: true,
            petnames: HashMap::new(),
            nicknames: HashMap::new(),
        }
    }

    /// Seeds `channel`'s transcript from persisted history loaded by the
    /// caller (see storage.rs). A no-op if `channel` isn't currently
    /// joined. Intended to be called once per joined channel right after
    /// `AppState::new`, before the event loop starts.
    pub fn load_history(&mut self, channel: &str, messages: Vec<ChatMessage>) {
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

    pub fn active(&self) -> &Channel {
        &self.channels[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Channel {
        &mut self.channels[self.active]
    }

    fn channel_mut(&mut self, name: &str) -> Option<&mut Channel> {
        self.channels.iter_mut().find(|c| c.name == name)
    }

    /// Pure key handling: no I/O, no network. Returns the action to
    /// perform when `Enter` sends a message or runs a command, so the
    /// caller can talk to the network layer.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<InputAction> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Char('a') if ctrl => self.move_home(),
            KeyCode::Char('e') if ctrl => self.move_end(),
            KeyCode::Char('u') if ctrl => self.delete_to_start(),
            KeyCode::Char('k') if ctrl => self.delete_to_end(),
            KeyCode::Char('w') if ctrl => self.delete_word_backward(),
            KeyCode::Enter => return self.submit_input(),
            KeyCode::Backspace => self.delete_backward(),
            KeyCode::Delete => self.delete_forward(),
            KeyCode::Home => self.move_home(),
            KeyCode::End => self.move_end(),
            KeyCode::Left => self.move_left(),
            KeyCode::Right => self.move_right(),
            KeyCode::Up => {
                let channel = self.active_mut();
                channel.scroll = channel.scroll.saturating_add(1);
            }
            KeyCode::Down => {
                let channel = self.active_mut();
                channel.scroll = channel.scroll.saturating_sub(1);
            }
            KeyCode::Tab => self.switch_channel(1),
            KeyCode::BackTab => self.switch_channel(-1),
            KeyCode::Char(c) => self.insert_char(c),
            _ => {}
        }
        None
    }

    /// Applies a network event: updates presence, pushes a deduped incoming
    /// message, records a peer's broadcast nickname, or reports a channel
    /// join's outcome. Returns the channel name and message when `event`
    /// was a `NetEvent::Received` that resulted in a newly-accepted
    /// (non-duplicate) message, so the caller can persist it (see
    /// storage.rs) -- `None` for every other event, including a duplicate
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
                match self.channels.iter().position(|c| c.name == name) {
                    Some(index) => self.active = index,
                    None => {
                        self.channels.push(Channel::new(name));
                        self.active = self.channels.len() - 1;
                    }
                }
                self.active_mut().has_unread = false;
                None
            }
            NetEvent::JoinFailed(name, error) => {
                self.push_system(format!("failed to join #{name}: {error}"));
                None
            }
            // A peer's (re-)announced broadcast nickname -- see
            // features.md's "Broadcast nicknames". Simply overwrite
            // whatever we last had for `sender`, so `display_name` always
            // reflects the most recently seen one.
            NetEvent::Identity(_channel, identity) => {
                self.nicknames.insert(identity.sender, identity.nickname);
                None
            }
            // Handled in main.rs before reaching here: `Announce` triggers
            // `Net::sync_history`, `HistoryFetched`'s payload goes through
            // `merge_history` above, and `PeerAddressLearned` is persisted
            // to the channel registry (see channel_registry.rs). All three
            // are network-sync/reconnection bookkeeping, not transcript or
            // presence state.
            NetEvent::Announce(..)
            | NetEvent::HistoryFetched(..)
            | NetEvent::PeerAddressLearned(..) => None,
        }
    }

    /// Removes `channel`'s tab and all its in-memory state (transcript,
    /// presence, dedupe buffers) -- called once the caller has actually
    /// torn down its gossip subscription and persisted state (see
    /// `InputAction::Leave`). Adjusts `active` to keep pointing at the
    /// same tab if it was after the removed one, or lands on the
    /// previous tab if the active tab itself was the one removed. A
    /// no-op if `channel` isn't currently joined.
    ///
    /// # Panics
    /// Panics if `channel` is leyline's only joined channel -- `AppState`
    /// always needs at least one (see `AppState::new`), and `run_leave`
    /// is the only caller that decides whether to request a removal, so
    /// it must already refuse this case before ever returning
    /// `InputAction::Leave`.
    pub fn remove_channel(&mut self, channel: &str) {
        let Some(index) = self.channels.iter().position(|c| c.name == channel) else {
            return;
        };
        assert!(
            self.channels.len() > 1,
            "must not remove leyline's only channel"
        );
        self.channels.remove(index);
        if self.active > index {
            self.active -= 1;
        } else if self.active == index {
            self.active = self.active.min(self.channels.len().saturating_sub(1));
        }
        self.push_system(format!("left #{channel}"));
    }

    /// Displays a sender as "you" for our own id; their pet name if one
    /// has been assigned via `/alias`; else their last broadcast nickname
    /// from `/nick`, suffixed with their hex prefix since a nickname is
    /// spoofable and not unique (features.md's "Broadcast nicknames"); or,
    /// absent both, a shortened hex id on its own.
    pub fn display_name(&self, sender: &[u8; 32]) -> String {
        if *sender == self.self_id {
            "you".to_string()
        } else if let Some(name) = self.petnames.get(sender) {
            name.clone()
        } else if let Some(nickname) = self.nicknames.get(sender) {
            format!("{nickname} ({})", hex_prefix(sender))
        } else {
            hex_prefix(sender)
        }
    }

    /// Appends a local-only system notice to the active channel's
    /// transcript (invite output, command errors) -- never sent over the
    /// network.
    pub fn push_system(&mut self, text: impl Into<String>) {
        self.active_mut().push(TranscriptLine::System(text.into()));
    }

    /// Moves `active` forward (`step = 1`) or backward (`step = -1`)
    /// through `channels`, wrapping, and clears the new active channel's
    /// unread flag.
    fn switch_channel(&mut self, step: isize) {
        if self.channels.len() <= 1 {
            return;
        }
        let len = self.channels.len() as isize;
        let next = (self.active as isize + step).rem_euclid(len) as usize;
        self.active = next;
        self.active_mut().has_unread = false;
    }

    fn submit_input(&mut self) -> Option<InputAction> {
        if self.input.trim().is_empty() {
            return None;
        }
        let text = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.active_mut().scroll = 0;

        if let Some(rest) = text.strip_prefix('/') {
            return self.run_command(rest.trim());
        }

        let channel = self.active().name.clone();
        let message = compose_message(self.self_id, &text);
        self.active_mut().remember_seen(message.id);
        self.active_mut()
            .push(TranscriptLine::Chat(message.clone()));
        Some(InputAction::Send(channel, message))
    }

    /// Parses a `/command [args]` line (the text after the leading `/`,
    /// already trimmed). Recognized commands may still return `None` after
    /// pushing a system notice (e.g. usage errors) -- only `/join`,
    /// `/invite`, and a successful `/alias` or `/nick` need the caller to
    /// actually do anything.
    fn run_command(&mut self, command: &str) -> Option<InputAction> {
        let (name, arg) = command.split_once(' ').unwrap_or((command, ""));
        match name {
            "join" => {
                let arg = arg.trim();
                if arg.is_empty() {
                    self.push_system("usage: /join <channel-name-or-ticket>");
                    None
                } else {
                    Some(InputAction::Join(arg.to_string()))
                }
            }
            "invite" => Some(InputAction::Invite(self.active().name.clone())),
            "alias" => self.run_alias(arg.trim()),
            "nick" => self.run_nick(arg.trim()),
            "leave" => self.run_leave(arg.trim()),
            "who" => self.run_who(),
            "search" => self.run_search(arg.trim()),
            "hints" => self.run_hints(),
            "help" => self.run_help(),
            _ => {
                self.push_system(format!("unknown command: /{name}"));
                None
            }
        }
    }

    /// Handles `/help`: pushes the full command/keybinding reference as a
    /// system notice. The input box's border only shows a short hint (see
    /// `ui::render_input`) to stay uncluttered, so this is where that
    /// detail actually lives.
    fn run_help(&mut self) -> Option<InputAction> {
        self.push_system(
            "commands: /join <name|ticket>, /invite, /leave [channel], /who, \
             /alias <hex-prefix> <name>, /nick <name>, /search <term>, /hints, \
             /help -- keys: Tab/Shift+Tab switch channels, Up/Down scroll, \
             Esc/Ctrl+C quit",
        );
        None
    }

    /// Handles `/hints`: toggles the sidebar's command hints panel (see
    /// `ui::render_sidebar`) and reports the new state as a system
    /// notice. Purely local UI state -- no `InputAction` needed, unlike
    /// commands that require the caller to do network I/O.
    fn run_hints(&mut self) -> Option<InputAction> {
        self.show_hints = !self.show_hints;
        let state = if self.show_hints { "shown" } else { "hidden" };
        self.push_system(format!("command hints {state}"));
        None
    }

    /// Handles `/alias <hex-prefix> <name>`: `arg` is everything after
    /// `/alias ` (already trimmed). Splits it into the hex-prefix and the
    /// (possibly multi-word) name to assign, resolves the prefix via
    /// `resolve_id`, and on success updates `petnames` immediately -- for
    /// instant UI feedback -- while returning an `InputAction::Alias` for
    /// the caller to persist (see contacts.rs). Usage and resolution
    /// errors are reported as a system notice instead.
    fn run_alias(&mut self, arg: &str) -> Option<InputAction> {
        let Some((prefix, name)) = arg.split_once(' ') else {
            self.push_system("usage: /alias <hex-prefix> <name>");
            return None;
        };
        let name = name.trim();
        if name.is_empty() {
            self.push_system("usage: /alias <hex-prefix> <name>");
            return None;
        }
        match self.resolve_id(prefix) {
            Ok(id) => {
                self.petnames.insert(id, name.to_string());
                self.push_system(format!("aliased {} as {name}", hex_id(&id)));
                Some(InputAction::Alias(id, name.to_string()))
            }
            Err(err) => {
                self.push_system(err);
                None
            }
        }
    }

    /// Handles `/nick <name>`: `arg` is everything after `/nick ` (already
    /// trimmed). Unlike `/alias`, there's no id to resolve -- a `/nick` is
    /// about our own identity, not a peer's -- so a bare non-empty name is
    /// all that's needed. Reports the outcome as a system notice either
    /// way, and returns an `InputAction::Nick` for the caller to broadcast
    /// (see `net::Net::set_nickname`) on success.
    fn run_nick(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system("usage: /nick <name>");
            return None;
        }
        self.push_system(format!(
            "nickname set to {arg} (broadcast to joined channels)"
        ));
        Some(InputAction::Nick(arg.to_string()))
    }

    /// Handles `/leave [channel]`: `arg` is everything after `/leave `
    /// (already trimmed), naming the channel to leave, or empty to leave
    /// the currently active one (mirroring `/invite`). Refuses -- with a
    /// system notice, returning `None` -- to leave a channel that isn't
    /// currently joined, or leyline's only remaining joined channel,
    /// since `AppState` always needs at least one (see `AppState::new`).
    /// On success, returns an `InputAction::Leave` for the caller to tear
    /// down; the tab itself is only removed once that's done, via
    /// `remove_channel`.
    fn run_leave(&mut self, arg: &str) -> Option<InputAction> {
        let target = if arg.is_empty() {
            self.active().name.clone()
        } else {
            arg.to_string()
        };
        if !self.channels.iter().any(|c| c.name == target) {
            self.push_system(format!("not currently in #{target}"));
            return None;
        }
        if self.channels.len() <= 1 {
            self.push_system("cannot leave your only channel");
            return None;
        }
        Some(InputAction::Leave(target))
    }

    /// Handles `/who`: lists the active channel's current peers, one per
    /// system notice, each showing their full endpoint id plus their
    /// broadcast nickname and/or local alias if set (see `who_line`).
    /// Unlike `display_name`, which blends petname/nickname/hex-prefix
    /// into a single string with a precedence order for compact display
    /// elsewhere, `/who` exists specifically to show all three fields
    /// explicitly (features.md's "`/who`" entry). Takes no argument --
    /// like `/invite`, it always reports on the active channel. Purely
    /// local, like `/hints`/`/help`: no `InputAction` needed.
    fn run_who(&mut self) -> Option<InputAction> {
        let channel = self.active();
        let name = channel.name.clone();
        let peers = channel.peers.clone();
        if peers.is_empty() {
            self.push_system(format!("no peers currently in #{name}"));
            return None;
        }
        self.push_system(format!("peers in #{name} ({}):", peers.len()));
        for id in &peers {
            let line = self.who_line(id);
            self.push_system(line);
        }
        None
    }

    /// Formats one `/who` line for `id`: its full hex id, plus `nickname:
    /// <n>` and/or `alias: <a>` in parentheses for whichever of
    /// `nicknames`/`petnames` actually have an entry for it -- both are
    /// shown when both are set, and neither is invented when absent,
    /// unlike `display_name`'s single-string fallback chain.
    fn who_line(&self, id: &[u8; 32]) -> String {
        let mut details = Vec::new();
        if let Some(nickname) = self.nicknames.get(id) {
            details.push(format!("nickname: {nickname}"));
        }
        if let Some(alias) = self.petnames.get(id) {
            details.push(format!("alias: {alias}"));
        }
        if details.is_empty() {
            hex_id(id)
        } else {
            format!("{} ({})", hex_id(id), details.join(", "))
        }
    }

    /// Handles `/search <term>`: `arg` is everything after `/search `
    /// (already trimmed). With no argument, clears an active search filter
    /// -- or shows a usage hint if none is active -- mirroring `/hints`'s
    /// toggle shape. Otherwise scans the active channel's loaded transcript
    /// synchronously for an instant result (`Channel::search_loaded`),
    /// stores it as the channel's active search, and returns an
    /// `InputAction::Search` so the caller can extend it with a background
    /// scan of history older than what's loaded (see
    /// `search::run_on_disk_scan` and `AppState::apply_search_outcome`).
    fn run_search(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            if self.active_mut().search.take().is_some() {
                self.push_system("search cleared");
            } else {
                self.push_system("usage: /search <term>");
            }
            return None;
        }
        let term_lower = arg.to_lowercase();
        let messages = self.active().search_loaded(&term_lower);
        let count = messages.len();
        self.active_mut().search = Some(SearchResults {
            term: arg.to_string(),
            messages,
            pending: true,
        });
        self.push_system(format!(
            "search: {count} match(es) so far for '{arg}' (scanning full history...)"
        ));
        Some(InputAction::Search {
            channel: self.active().name.clone(),
            term: arg.to_string(),
        })
    }

    /// Resolves a hex-prefix (case-insensitive, as typed after `/alias`)
    /// to exactly one id in `known_ids`. Errors with a user-facing message
    /// if the prefix isn't valid hex, matches no known id, or matches more
    /// than one -- this is what makes `/alias` "do nothing for a peer you
    /// haven't met yet" rather than guessing.
    fn resolve_id(&self, prefix: &str) -> Result<[u8; 32], String> {
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
    fn known_ids(&self) -> Vec<[u8; 32]> {
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

    fn char_count(&self) -> usize {
        self.input.chars().count()
    }

    /// Byte offset in `self.input` corresponding to a character index.
    ///
    /// Needed because `String` indexing/mutation is byte-based, but the
    /// cursor is tracked as a character index so editing works correctly
    /// with multi-byte UTF-8 input.
    fn byte_index(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.input.len())
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_count());
    }

    fn move_home(&mut self) {
        self.cursor = 0;
    }

    fn move_end(&mut self) {
        self.cursor = self.char_count();
    }

    fn insert_char(&mut self, c: char) {
        let idx = self.byte_index(self.cursor);
        self.input.insert(idx, c);
        self.cursor += 1;
    }

    /// Backspace: deletes the character before the cursor.
    fn delete_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_index(self.cursor);
        let start = self.byte_index(self.cursor - 1);
        self.input.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// Delete: deletes the character at the cursor.
    fn delete_forward(&mut self) {
        if self.cursor >= self.char_count() {
            return;
        }
        let start = self.byte_index(self.cursor);
        let end = self.byte_index(self.cursor + 1);
        self.input.replace_range(start..end, "");
    }

    /// Ctrl+U: deletes from the start of the line to the cursor.
    fn delete_to_start(&mut self) {
        let end = self.byte_index(self.cursor);
        self.input.replace_range(0..end, "");
        self.cursor = 0;
    }

    /// Ctrl+K: deletes from the cursor to the end of the line.
    fn delete_to_end(&mut self) {
        let start = self.byte_index(self.cursor);
        self.input.truncate(start);
    }

    /// Ctrl+W: deletes the word immediately before the cursor.
    fn delete_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.input.chars().collect();
        let mut idx = self.cursor;
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        while idx > 0 && !chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        let start = self.byte_index(idx);
        let end = self.byte_index(self.cursor);
        self.input.replace_range(start..end, "");
        self.cursor = idx;
    }
}

/// Shortened hex id used to display peers we don't have a nickname for.
fn hex_prefix(bytes: &[u8; 32]) -> String {
    bytes[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Full hex id, e.g. for display in the header so it can be copied into
/// another instance's `--connect` flag.
pub fn hex_id(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn compose_message(sender: [u8; 32], text: &str) -> ChatMessage {
    ChatMessage {
        v: 1,
        id: rand::random(),
        sender,
        ts_unix_ms: now_unix_ms(),
        text: text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::IdentityAnnounce;

    const SELF_ID: [u8; 32] = [9; 32];
    const PEER_ID: [u8; 32] = [7; 32];
    const PEER_ID_2: [u8; 32] = [8; 32];

    fn app() -> AppState {
        AppState::new(SELF_ID, vec!["general".to_string()], "general")
    }

    fn multi_channel_app() -> AppState {
        AppState::new(
            SELF_ID,
            vec!["general".to_string(), "random".to_string()],
            "random",
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Unwraps a `TranscriptLine::Chat`'s inner message, panicking with a
    /// helpful message if the line is actually a `TranscriptLine::System`.
    fn as_chat(line: &TranscriptLine) -> &ChatMessage {
        match line {
            TranscriptLine::Chat(message) => message,
            TranscriptLine::System(text) => {
                panic!("expected a chat message, got system line: {text:?}")
            }
        }
    }

    /// Unwraps a `TranscriptLine::System`'s inner text, panicking if the
    /// line is actually a `TranscriptLine::Chat`.
    fn as_system(line: &TranscriptLine) -> &str {
        match line {
            TranscriptLine::System(text) => text,
            TranscriptLine::Chat(_) => panic!("expected a system line, got a chat message"),
        }
    }

    /// Types `text` character-by-character then presses `Enter`, returning
    /// whatever action (if any) that produced -- shared by the `/alias`
    /// tests below, which all follow this shape.
    fn submit(app: &mut AppState, text: &str) -> Option<InputAction> {
        for c in text.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter))
    }

    #[test]
    fn typing_composes_input() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        assert_eq!(app.input, "hi");
    }

    #[test]
    fn backspace_edits_input() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.input, "h");
    }

    #[test]
    fn enter_sends_and_clears_input() {
        let mut app = app();
        let before = app.active().messages.len();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        let sent = app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.input, "");
        assert_eq!(app.active().messages.len(), before + 1);
        let last = as_chat(app.active().messages.back().unwrap());
        assert_eq!(last.text, "hi");
        assert_eq!(last.sender, SELF_ID);
        match sent.expect("enter with non-empty input returns an action") {
            InputAction::Send(channel, message) => {
                assert_eq!(channel, "general");
                assert_eq!(message.text, "hi");
                assert_eq!(message.sender, SELF_ID);
            }
            _ => panic!("expected InputAction::Send"),
        }
    }

    #[test]
    fn empty_enter_does_not_send() {
        let mut app = app();
        let before = app.active().messages.len();
        let sent = app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.active().messages.len(), before);
        assert!(sent.is_none());
    }

    #[test]
    fn esc_quits() {
        let mut app = app();
        app.handle_key(key(KeyCode::Esc));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn display_name_maps_self_and_falls_back_to_hex_for_others() {
        let app = app();
        assert_eq!(app.display_name(&SELF_ID), "you");
        assert_eq!(app.display_name(&PEER_ID), hex_prefix(&PEER_ID));
    }

    #[test]
    fn display_name_uses_petname_when_one_is_set() {
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        assert_eq!(app.display_name(&PEER_ID), "Alice");
    }

    #[test]
    fn display_name_uses_nickname_when_no_petname_is_set() {
        let mut app = app();
        let result = app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "Alice".to_string(),
            },
        ));
        assert!(result.is_none());
        assert_eq!(
            app.display_name(&PEER_ID),
            format!("Alice ({})", hex_prefix(&PEER_ID)),
            "a broadcast nickname must be shown alongside the hex id, since it's spoofable"
        );
    }

    #[test]
    fn display_name_prefers_petname_over_nickname() {
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Bob".to_string())]));
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "Alice".to_string(),
            },
        ));
        assert_eq!(
            app.display_name(&PEER_ID),
            "Bob",
            "a pinned local petname must win over a peer's own broadcast nickname"
        );
    }

    #[test]
    fn identity_event_overwrites_previous_nickname_for_the_same_sender() {
        let mut app = app();
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "Alice".to_string(),
            },
        ));
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "Alicia".to_string(),
            },
        ));
        assert_eq!(
            app.display_name(&PEER_ID),
            format!("Alicia ({})", hex_prefix(&PEER_ID)),
            "display_name must reflect the last-seen nickname, not the first"
        );
    }

    #[test]
    fn alias_assigns_a_petname_for_a_currently_online_peer() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));

        let full_id = hex_id(&PEER_ID);
        let action = submit(&mut app, &format!("/alias {} Alice", &full_id[..8]));

        match action.expect("a successful /alias returns an action") {
            InputAction::Alias(id, name) => {
                assert_eq!(id, PEER_ID);
                assert_eq!(name, "Alice");
            }
            _ => panic!("expected InputAction::Alias"),
        }
        assert_eq!(app.display_name(&PEER_ID), "Alice");
    }

    #[test]
    fn alias_matches_a_peer_known_only_from_a_past_message() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hi"),
        ));

        let full_id = hex_id(&PEER_ID);
        let action = submit(&mut app, &format!("/alias {} Bob", &full_id[..8]));

        assert!(
            matches!(action, Some(InputAction::Alias(id, _)) if id == PEER_ID),
            "a peer seen only as a message sender must still be aliasable"
        );
    }

    #[test]
    fn alias_reports_an_error_for_an_unknown_prefix() {
        let mut app = app();
        let action = submit(&mut app, "/alias ffffffff Carol");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("no known peer"), "got: {last}");
    }

    #[test]
    fn alias_reports_an_error_for_an_ambiguous_prefix() {
        let mut app = app();
        let mut peer_a = [0xAB; 32];
        let mut peer_b = [0xAB; 32];
        peer_a[4] = 0x01;
        peer_b[4] = 0x02;
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), peer_a));
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), peer_b));

        // Both ids share this 4-byte (8 hex char) prefix.
        let full_id = hex_id(&peer_a);
        let action = submit(&mut app, &format!("/alias {} Dave", &full_id[..8]));

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("multiple peers"), "got: {last}");
    }

    #[test]
    fn alias_rejects_a_non_hex_prefix() {
        let mut app = app();
        let action = submit(&mut app, "/alias not-hex Eve");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("invalid hex prefix"), "got: {last}");
    }

    #[test]
    fn alias_usage_error_when_name_is_missing() {
        let mut app = app();
        let action = submit(&mut app, "/alias ffffffff");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"));
    }

    #[test]
    fn nick_command_returns_action_and_pushes_system_notice() {
        let mut app = app();
        let action = submit(&mut app, "/nick Alice");
        match action.expect("a successful /nick returns an action") {
            InputAction::Nick(name) => assert_eq!(name, "Alice"),
            _ => panic!("expected InputAction::Nick"),
        }
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("Alice"), "got: {last}");
    }

    #[test]
    fn nick_usage_error_when_name_is_missing() {
        let mut app = app();
        let action = submit(&mut app, "/nick");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"));
    }

    #[test]
    fn left_right_move_cursor_without_editing() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        assert_eq!(app.cursor, 2);
        app.handle_key(key(KeyCode::Left));
        assert_eq!(app.cursor, 1);
        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.cursor, 2);
        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.cursor, 2);
        assert_eq!(app.input, "hi");
    }

    #[test]
    fn insert_in_the_middle_of_input() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(key(KeyCode::Left));
        app.handle_key(key(KeyCode::Char('X')));
        assert_eq!(app.input, "hXi");
        assert_eq!(app.cursor, 2);
    }

    #[test]
    fn home_and_end_jump_cursor() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(key(KeyCode::Home));
        assert_eq!(app.cursor, 0);
        app.handle_key(key(KeyCode::End));
        assert_eq!(app.cursor, 2);
    }

    #[test]
    fn ctrl_a_and_ctrl_e_jump_cursor() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(app.cursor, 0);
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(app.cursor, 2);
    }

    #[test]
    fn delete_key_removes_char_at_cursor() {
        let mut app = app();
        for c in ['h', 'i', '!'] {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Home));
        app.handle_key(key(KeyCode::Delete));
        assert_eq!(app.input, "i!");
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn ctrl_u_deletes_to_start() {
        let mut app = app();
        for c in ['h', 'i', '!'] {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Left));
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.input, "!");
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn ctrl_k_deletes_to_end() {
        let mut app = app();
        for c in ['h', 'i', '!'] {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Home));
        app.handle_key(key(KeyCode::Right));
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(app.input, "h");
        assert_eq!(app.cursor, 1);
    }

    #[test]
    fn ctrl_w_deletes_word_backward() {
        let mut app = app();
        for c in "hello world".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.input, "hello ");
        assert_eq!(app.cursor, 6);
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.input, "");
        assert_eq!(app.cursor, 0);
    }

    #[test]
    fn peer_joined_and_left_update_peers() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert_eq!(app.active().peers, vec![PEER_ID]);
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert_eq!(
            app.active().peers,
            vec![PEER_ID],
            "joining twice should not duplicate"
        );
        app.handle_net_event(NetEvent::PeerLeft("general".to_string(), PEER_ID));
        assert!(app.active().peers.is_empty());
    }

    #[test]
    fn received_message_is_deduped_by_id() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hi from a peer");
        let before = app.active().messages.len();
        app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
        assert_eq!(app.active().messages.len(), before + 1);
        // Gossip is best-effort and can redeliver -- the same id must not
        // be shown twice.
        app.handle_net_event(NetEvent::Received("general".to_string(), message));
        assert_eq!(app.active().messages.len(), before + 1);
    }

    #[test]
    fn tab_and_backtab_cycle_channels_and_clear_unread() {
        let mut app = multi_channel_app();
        assert_eq!(app.active, 1, "starts on the last-joined channel");

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.active, 0, "Tab wraps around");
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.active, 1);

        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.active, 0, "Shift+Tab moves backward");
        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.active, 1, "and wraps the other way");
    }

    #[test]
    fn switching_into_a_channel_clears_its_unread_flag() {
        let mut app = multi_channel_app();
        app.active = 1; // on "random"
        app.channels[0].has_unread = true; // "general" has unread activity

        app.handle_key(key(KeyCode::Tab)); // wraps to "general"
        assert_eq!(app.active, 0);
        assert!(!app.active().has_unread);
    }

    #[test]
    fn received_message_on_inactive_channel_marks_unread_not_active() {
        let mut app = multi_channel_app();
        app.active = 1; // "random" is active; "general" is not

        let message = compose_message(PEER_ID, "hello");
        app.handle_net_event(NetEvent::Received("general".to_string(), message));

        assert!(app.channels[0].has_unread, "inactive channel gets flagged");
        assert!(!app.channels[1].has_unread, "active channel is untouched");
    }

    #[test]
    fn received_message_on_active_channel_does_not_mark_unread() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hello");
        app.handle_net_event(NetEvent::Received("general".to_string(), message));
        assert!(!app.active().has_unread);
    }

    #[test]
    fn messages_and_peers_are_isolated_per_channel() {
        let mut app = multi_channel_app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "only in general"),
        ));

        assert_eq!(app.channels[0].peers, vec![PEER_ID]);
        assert_eq!(app.channels[0].messages.len(), 1);
        assert!(app.channels[1].peers.is_empty());
        assert!(app.channels[1].messages.is_empty());
    }

    #[test]
    fn slash_join_with_arg_returns_join_action() {
        let mut app = app();
        for c in "/join project-x".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let action = app.handle_key(key(KeyCode::Enter));
        match action.expect("non-empty /join returns an action") {
            InputAction::Join(arg) => assert_eq!(arg, "project-x"),
            _ => panic!("expected InputAction::Join"),
        }
        assert_eq!(app.input, "");
    }

    #[test]
    fn slash_join_without_arg_shows_usage_and_returns_none() {
        let mut app = app();
        for c in "/join".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"));
    }

    #[test]
    fn unknown_command_shows_error_and_returns_none() {
        let mut app = app();
        for c in "/bogus".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "unknown command: /bogus");
    }

    #[test]
    fn help_command_pushes_a_system_notice_and_returns_none() {
        let mut app = app();
        let action = submit(&mut app, "/help");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("commands:"), "got: {last}");
    }

    #[test]
    fn hints_are_shown_by_default() {
        assert!(app().show_hints);
    }

    #[test]
    fn slash_hints_toggles_visibility_and_reports_state() {
        let mut app = app();

        submit(&mut app, "/hints");
        assert!(!app.show_hints);
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "command hints hidden");

        submit(&mut app, "/hints");
        assert!(app.show_hints);
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "command hints shown");
    }

    #[test]
    fn slash_invite_returns_invite_action_for_active_channel() {
        let mut app = multi_channel_app();
        for c in "/invite".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let action = app.handle_key(key(KeyCode::Enter));
        match action.expect("/invite returns an action") {
            InputAction::Invite(channel) => assert_eq!(channel, "random"),
            _ => panic!("expected InputAction::Invite"),
        }
    }

    #[test]
    fn slash_leave_with_no_arg_leaves_the_active_channel() {
        let mut app = multi_channel_app();
        let action = submit(&mut app, "/leave");
        match action.expect("/leave returns an action") {
            InputAction::Leave(channel) => assert_eq!(channel, "random"),
            _ => panic!("expected InputAction::Leave"),
        }
    }

    #[test]
    fn slash_leave_with_a_name_leaves_that_channel_even_if_inactive() {
        let mut app = multi_channel_app();
        assert_eq!(app.active().name, "random");
        let action = submit(&mut app, "/leave general");
        match action.expect("/leave returns an action") {
            InputAction::Leave(channel) => assert_eq!(channel, "general"),
            _ => panic!("expected InputAction::Leave"),
        }
    }

    #[test]
    fn slash_leave_refuses_an_unjoined_channel() {
        let mut app = multi_channel_app();
        let action = submit(&mut app, "/leave nonexistent");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "not currently in #nonexistent");
    }

    #[test]
    fn slash_leave_refuses_the_only_channel() {
        let mut app = app();
        let action = submit(&mut app, "/leave");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "cannot leave your only channel");
    }

    #[test]
    fn slash_who_reports_no_peers_when_channel_is_empty() {
        let mut app = app();
        let action = submit(&mut app, "/who");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "no peers currently in #general");
    }

    #[test]
    fn slash_who_lists_a_peer_with_no_nickname_or_alias() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        let action = submit(&mut app, "/who");
        assert!(action.is_none());
        let messages: Vec<&TranscriptLine> = app.active().messages.iter().collect();
        let len = messages.len();
        assert_eq!(as_system(messages[len - 2]), "peers in #general (1):");
        assert_eq!(as_system(messages[len - 1]), hex_id(&PEER_ID));
    }

    #[test]
    fn slash_who_shows_nickname_when_set() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "alice".to_string(),
            },
        ));
        submit(&mut app, "/who");
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, format!("{} (nickname: alice)", hex_id(&PEER_ID)));
    }

    #[test]
    fn slash_who_shows_alias_when_set() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.load_contacts(HashMap::from([(PEER_ID, "Bob".to_string())]));
        submit(&mut app, "/who");
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, format!("{} (alias: Bob)", hex_id(&PEER_ID)));
    }

    #[test]
    fn slash_who_shows_both_nickname_and_alias_when_both_are_set() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.load_contacts(HashMap::from([(PEER_ID, "Bob".to_string())]));
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "alice".to_string(),
            },
        ));
        submit(&mut app, "/who");
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(
            last,
            format!("{} (nickname: alice, alias: Bob)", hex_id(&PEER_ID))
        );
    }

    #[test]
    fn slash_who_lists_every_peer_in_the_active_channel() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID_2));
        submit(&mut app, "/who");
        let messages: Vec<&TranscriptLine> = app.active().messages.iter().collect();
        let len = messages.len();
        assert_eq!(as_system(messages[len - 3]), "peers in #general (2):");
        assert_eq!(as_system(messages[len - 2]), hex_id(&PEER_ID));
        assert_eq!(as_system(messages[len - 1]), hex_id(&PEER_ID_2));
    }

    #[test]
    fn slash_who_only_lists_the_active_channels_peers() {
        let mut app = multi_channel_app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert_eq!(app.active().name, "random");
        submit(&mut app, "/who");
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(
            last, "no peers currently in #random",
            "a peer in another joined channel must not show up"
        );
    }

    #[test]
    fn remove_channel_switches_active_to_the_previous_tab_when_removing_the_active_one() {
        let mut app = multi_channel_app();
        assert_eq!(app.active().name, "random");
        app.remove_channel("random");
        assert_eq!(app.channels.len(), 1);
        assert_eq!(app.active().name, "general");
    }

    #[test]
    fn remove_channel_keeps_the_active_tab_selected_when_removing_a_different_one() {
        let mut app = multi_channel_app();
        assert_eq!(app.active().name, "random");
        app.remove_channel("general");
        assert_eq!(app.channels.len(), 1);
        assert_eq!(
            app.active().name,
            "random",
            "removing an earlier tab must not change which channel is active"
        );
    }

    #[test]
    fn remove_channel_pushes_a_left_system_notice() {
        let mut app = multi_channel_app();
        app.remove_channel("random");
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "left #random");
    }

    #[test]
    fn remove_channel_is_a_no_op_for_an_unknown_channel() {
        let mut app = multi_channel_app();
        app.remove_channel("nonexistent");
        assert_eq!(app.channels.len(), 2);
        assert_eq!(app.active().name, "random");
    }

    #[test]
    fn joined_event_adds_new_channel_and_switches_to_it() {
        let mut app = app();
        app.handle_net_event(NetEvent::Joined("project-x".to_string()));
        assert_eq!(app.channels.len(), 2);
        assert_eq!(app.active().name, "project-x");
    }

    #[test]
    fn joined_event_on_existing_channel_switches_without_duplicating() {
        let mut app = multi_channel_app();
        app.active = 0;
        app.handle_net_event(NetEvent::Joined("random".to_string()));
        assert_eq!(
            app.channels.len(),
            2,
            "must not duplicate an existing channel"
        );
        assert_eq!(app.active().name, "random");
    }

    #[test]
    fn lagged_pushes_system_message_on_the_affected_channel() {
        let mut app = multi_channel_app();
        app.active = 1; // "random" is active; "general" is not

        app.handle_net_event(NetEvent::Lagged("general".to_string()));

        let last = as_system(app.channels[0].messages.back().unwrap());
        assert!(last.contains("lagged"));
        assert!(
            app.channels[1].messages.is_empty(),
            "lag on another channel shouldn't touch the active one"
        );
    }

    #[test]
    fn join_failed_pushes_system_message_on_active_channel() {
        let mut app = app();
        app.handle_net_event(NetEvent::JoinFailed(
            "project-x".to_string(),
            "boom".to_string(),
        ));
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "failed to join #project-x: boom");
    }

    #[test]
    fn handle_net_event_returns_channel_and_message_for_a_new_received_message() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hello");
        let result =
            app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
        match result {
            Some((channel, returned)) => {
                assert_eq!(channel, "general");
                assert_eq!(returned, message);
            }
            None => panic!("expected Some for a newly-received message"),
        }
    }

    #[test]
    fn handle_net_event_returns_none_for_a_duplicate_received_message() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hello");
        app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
        let result = app.handle_net_event(NetEvent::Received("general".to_string(), message));
        assert!(result.is_none());
    }

    #[test]
    fn handle_net_event_returns_none_for_non_received_events() {
        let mut app = app();
        let result = app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert!(result.is_none());
    }

    #[test]
    fn load_history_seeds_transcript_and_dedupes_like_live_messages() {
        let mut app = app();
        let message = compose_message(PEER_ID, "from before restart");
        app.load_history("general", vec![message.clone()]);

        assert_eq!(app.active().messages.len(), 1);
        assert_eq!(as_chat(app.active().messages.back().unwrap()), &message);

        // A live redelivery of the same id must still be deduped.
        let before = app.active().messages.len();
        app.handle_net_event(NetEvent::Received("general".to_string(), message));
        assert_eq!(app.active().messages.len(), before);
    }

    #[test]
    fn load_history_on_an_unjoined_channel_is_a_no_op() {
        let mut app = app();
        app.load_history("nonexistent", vec![compose_message(PEER_ID, "hi")]);
        assert_eq!(app.active().messages.len(), 0);
    }

    fn dated_message(ts_unix_ms: u64, id: u64, sender: [u8; 32], text: &str) -> ChatMessage {
        ChatMessage {
            v: 1,
            id,
            sender,
            ts_unix_ms,
            text: text.to_string(),
        }
    }

    #[test]
    fn merge_history_dedupes_against_already_known_messages() {
        let mut app = app();
        let known = dated_message(100, 1, PEER_ID, "known");
        app.load_history("general", vec![known.clone()]);

        let new_message = dated_message(200, 2, PEER_ID, "new");
        let accepted = app.merge_history("general", vec![known, new_message.clone()]);

        assert_eq!(accepted, vec![new_message]);
        assert_eq!(app.active().messages.len(), 2);
    }

    #[test]
    fn merge_history_sorts_backfilled_messages_by_timestamp_not_append_order() {
        let mut app = app();
        let recent = dated_message(200, 1, PEER_ID, "recent");
        app.load_history("general", vec![recent.clone()]);

        let older = dated_message(100, 2, PEER_ID, "older");
        app.merge_history("general", vec![older.clone()]);

        let messages: Vec<&ChatMessage> = app.active().messages.iter().map(as_chat).collect();
        assert_eq!(
            messages,
            vec![&older, &recent],
            "an older backfilled message must sort before what was already there"
        );
    }

    #[test]
    fn merge_history_returns_nothing_when_everything_is_already_known() {
        let mut app = app();
        let message = dated_message(100, 1, PEER_ID, "hi");
        app.load_history("general", vec![message.clone()]);

        let accepted = app.merge_history("general", vec![message]);
        assert!(accepted.is_empty());
    }

    #[test]
    fn merge_history_on_an_unjoined_channel_is_a_no_op() {
        let mut app = app();
        let accepted = app.merge_history("nonexistent", vec![dated_message(100, 1, PEER_ID, "hi")]);
        assert!(accepted.is_empty());
        assert_eq!(app.active().messages.len(), 0);
    }

    #[test]
    fn merge_history_keeps_system_lines_after_the_resorted_chat_messages() {
        let mut app = app();
        app.push_system("earlier notice");
        let older = dated_message(100, 1, PEER_ID, "older");
        app.merge_history("general", vec![older.clone()]);

        let lines: Vec<&TranscriptLine> = app.active().messages.iter().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(as_chat(lines[0]), &older);
        assert_eq!(as_system(lines[1]), "earlier notice");
    }

    #[test]
    fn merge_history_enforces_the_scrollback_cap_evicting_the_oldest_first() {
        let mut app = app();
        let batch: Vec<ChatMessage> = (0..(MAX_SCROLLBACK as u64 + 10))
            .map(|i| dated_message(i, i, PEER_ID, "msg"))
            .collect();

        let accepted = app.merge_history("general", batch);

        assert_eq!(accepted.len(), MAX_SCROLLBACK + 10);
        assert_eq!(app.active().messages.len(), MAX_SCROLLBACK);
        let oldest_remaining = as_chat(app.active().messages.front().unwrap());
        assert_eq!(
            oldest_remaining.id, 10,
            "the 10 oldest messages must have been evicted to stay within the cap"
        );
    }

    #[test]
    fn slash_search_without_arg_and_no_active_filter_shows_usage() {
        let mut app = app();
        let action = submit(&mut app, "/search");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"), "got: {last}");
    }

    #[test]
    fn slash_search_finds_matches_in_loaded_history_and_returns_an_action() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hello world"),
        ));
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "goodbye"),
        ));

        let action = submit(&mut app, "/search world");

        match action.expect("a successful /search returns an action") {
            InputAction::Search { channel, term } => {
                assert_eq!(channel, "general");
                assert_eq!(term, "world");
            }
            _ => panic!("expected InputAction::Search"),
        }
        let search = app
            .active()
            .search
            .as_ref()
            .expect("search should be active");
        assert_eq!(search.messages.len(), 1);
        assert_eq!(search.messages[0].text, "hello world");
        assert!(search.pending, "the on-disk scan hasn't reported back yet");
    }

    #[test]
    fn slash_search_is_case_insensitive() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "Hello World"),
        ));

        submit(&mut app, "/search WORLD");

        let search = app
            .active()
            .search
            .as_ref()
            .expect("search should be active");
        assert_eq!(search.messages.len(), 1);
    }

    #[test]
    fn slash_search_with_no_matches_still_activates_an_empty_search() {
        let mut app = app();
        let action = submit(&mut app, "/search nonexistent");
        assert!(action.is_some());
        let search = app
            .active()
            .search
            .as_ref()
            .expect("search should be active");
        assert!(search.messages.is_empty());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("0 match"), "got: {last}");
    }

    #[test]
    fn slash_search_with_no_arg_clears_an_active_filter() {
        let mut app = app();
        submit(&mut app, "/search hello");
        assert!(app.active().search.is_some());

        let action = submit(&mut app, "/search");

        assert!(action.is_none());
        assert!(app.active().search.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "search cleared");
    }

    #[test]
    fn apply_search_outcome_merges_on_disk_matches_into_existing_results() {
        let mut app = app();
        submit(&mut app, "/search hello");
        let older = dated_message(50, 99, PEER_ID, "hello from the past");

        app.apply_search_outcome(SearchOutcome {
            channel: "general".to_string(),
            term: "hello".to_string(),
            messages: vec![older.clone()],
        });

        let search = app.active().search.as_ref().unwrap();
        assert_eq!(search.messages, vec![older]);
        assert!(!search.pending);
    }

    #[test]
    fn apply_search_outcome_deduplicates_messages_already_in_the_snapshot() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hello there"),
        ));
        let action = submit(&mut app, "/search hello");
        let term = match action.expect("search returns an action") {
            InputAction::Search { term, .. } => term,
            _ => panic!("expected InputAction::Search"),
        };
        let already_shown = app.active().search.as_ref().unwrap().messages[0].clone();

        app.apply_search_outcome(SearchOutcome {
            channel: "general".to_string(),
            term,
            messages: vec![already_shown.clone()],
        });

        let search = app.active().search.as_ref().unwrap();
        assert_eq!(search.messages, vec![already_shown]);
    }

    #[test]
    fn apply_search_outcome_ignores_a_stale_result_from_a_superseded_search() {
        let mut app = app();
        submit(&mut app, "/search first");
        submit(&mut app, "/search second");

        app.apply_search_outcome(SearchOutcome {
            channel: "general".to_string(),
            term: "first".to_string(),
            messages: vec![dated_message(1, 1, PEER_ID, "first match")],
        });

        let search = app.active().search.as_ref().unwrap();
        assert_eq!(search.term, "second");
        assert!(search.messages.is_empty());
    }

    #[test]
    fn apply_search_outcome_is_a_no_op_for_an_unknown_channel() {
        let mut app = app();
        app.apply_search_outcome(SearchOutcome {
            channel: "nonexistent".to_string(),
            term: "hello".to_string(),
            messages: vec![dated_message(1, 1, PEER_ID, "hello")],
        });
        assert!(app.active().search.is_none());
    }
}
