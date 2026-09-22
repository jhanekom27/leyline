//! Owns all UI state and the pure logic that mutates it.
//!
//! Per concept.md's "one owner of UI state" principle: nothing here does
//! network or file I/O. `handle_key` is a pure function of the current
//! state and a keypress -- when the user sends a message or runs a `/join`
//! or `/invite` command, it returns an `InputAction` describing what the
//! caller should do, rather than reaching for I/O itself.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::message::ChatMessage;
use crate::net::NetEvent;

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
}

impl AppState {
    /// Builds the initial state for a session with the given identity and
    /// already-joined channel names (in join order). The last-joined
    /// channel starts active -- e.g. so `--join <ticket-for-project-x>`
    /// lands you in `project-x`, not `general`.
    ///
    /// # Panics
    /// Panics if `channel_names` is empty. `AppState` always has at least
    /// one channel, so `active` is always a valid index.
    pub fn new(self_id: [u8; 32], channel_names: Vec<String>) -> Self {
        assert!(
            !channel_names.is_empty(),
            "AppState must start with at least one channel"
        );
        let channels: Vec<Channel> = channel_names.into_iter().map(Channel::new).collect();
        let active = channels.len() - 1;
        Self {
            self_id,
            channels,
            active,
            input: String::new(),
            cursor: 0,
            should_quit: false,
        }
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
    /// message, or reports a channel join's outcome.
    pub fn handle_net_event(&mut self, event: NetEvent) {
        match event {
            NetEvent::PeerJoined(channel_name, id) => {
                if let Some(channel) = self.channel_mut(&channel_name)
                    && !channel.peers.contains(&id)
                {
                    channel.peers.push(id);
                }
            }
            NetEvent::PeerLeft(channel_name, id) => {
                if let Some(channel) = self.channel_mut(&channel_name) {
                    channel.peers.retain(|peer| *peer != id);
                }
            }
            NetEvent::Received(channel_name, message) => {
                let is_active = self.active().name == channel_name;
                if let Some(channel) = self.channel_mut(&channel_name)
                    && channel.remember_seen(message.id)
                {
                    channel.push(TranscriptLine::Chat(message));
                    if !is_active {
                        channel.has_unread = true;
                    }
                }
            }
            NetEvent::Lagged(channel_name) => {
                if let Some(channel) = self.channel_mut(&channel_name) {
                    channel.push(TranscriptLine::System(
                        "some messages may have been missed (lagged)".to_string(),
                    ));
                }
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
            }
            NetEvent::JoinFailed(name, error) => {
                self.push_system(format!("failed to join #{name}: {error}"));
            }
        }
    }

    /// Displays a sender as "you" for our own id, otherwise a shortened hex
    /// id -- the same fallback real, unnamed peers use until nicknames
    /// land.
    pub fn display_name(&self, sender: &[u8; 32]) -> String {
        if *sender == self.self_id {
            "you".to_string()
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
    /// pushing a system notice (e.g. usage errors) -- only `/join` and
    /// `/invite` need the caller to actually do anything.
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
            _ => {
                self.push_system(format!("unknown command: /{name}"));
                None
            }
        }
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

    const SELF_ID: [u8; 32] = [9; 32];
    const PEER_ID: [u8; 32] = [7; 32];

    fn app() -> AppState {
        AppState::new(SELF_ID, vec!["general".to_string()])
    }

    fn multi_channel_app() -> AppState {
        AppState::new(SELF_ID, vec!["general".to_string(), "random".to_string()])
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
}
