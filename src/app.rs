//! Owns all UI state and the pure logic that mutates it.
//!
//! Per concept.md's "one owner of UI state" principle: nothing here does
//! network or file I/O. `handle_key` is a pure function of the current
//! state and a keypress -- when the user sends a message it returns the
//! composed `ChatMessage` so the caller can hand it off to the network
//! layer, rather than reaching for I/O itself.

use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::message::ChatMessage;
use crate::net::NetEvent;

/// Cap on in-memory scrollback, per concept.md's guidance on bounding
/// render/memory cost in a long-lived session.
const MAX_SCROLLBACK: usize = 500;

/// Cap on how many recently-seen message ids we remember. Gossip is
/// best-effort and can redeliver, so incoming messages are deduped by id --
/// see concept.md's "Message wire format" section.
const MAX_SEEN_IDS: usize = 256;

/// All state needed to render the TUI and respond to input.
pub struct AppState {
    /// Our own identity, so we can tell our messages apart from peers'.
    pub self_id: [u8; 32],
    pub messages: VecDeque<ChatMessage>,
    pub peers: Vec<[u8; 32]>,
    pub input: String,
    pub cursor: usize,
    pub scroll: usize,
    pub should_quit: bool,
    /// Recently-seen message ids, oldest first, for incoming-message dedupe.
    seen_ids: VecDeque<u64>,
}

impl AppState {
    /// Builds the initial state for a session with the given identity.
    /// Messages and peers start empty and populate from real `NetEvent`s.
    pub fn new(self_id: [u8; 32]) -> Self {
        Self {
            self_id,
            messages: VecDeque::new(),
            peers: Vec::new(),
            input: String::new(),
            cursor: 0,
            scroll: 0,
            should_quit: false,
            seen_ids: VecDeque::new(),
        }
    }

    /// Pure key handling: no I/O, no network. Returns the composed message
    /// when `Enter` sends one, so the caller can broadcast it.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<ChatMessage> {
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
            KeyCode::Up => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Down => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Char(c) => self.insert_char(c),
            _ => {}
        }
        None
    }

    /// Applies a network event: updates presence, or pushes a deduped
    /// incoming message into the scrollback.
    pub fn handle_net_event(&mut self, event: NetEvent) {
        match event {
            NetEvent::PeerJoined(id) => {
                if !self.peers.contains(&id) {
                    self.peers.push(id);
                }
            }
            NetEvent::PeerLeft(id) => {
                self.peers.retain(|peer| *peer != id);
            }
            NetEvent::Received(message) => {
                if self.remember_seen(message.id) {
                    self.push_message(message);
                }
            }
            NetEvent::Lagged => {}
        }
    }

    /// Displays a sender as "you" for our own id, otherwise a shortened hex
    /// id -- the same fallback real, unnamed peers use until nicknames or
    /// presence info (build-order step 4) land.
    pub fn display_name(&self, sender: &[u8; 32]) -> String {
        if *sender == self.self_id {
            "you".to_string()
        } else {
            hex_prefix(sender)
        }
    }

    fn submit_input(&mut self) -> Option<ChatMessage> {
        if self.input.trim().is_empty() {
            return None;
        }
        let text = std::mem::take(&mut self.input);
        self.cursor = 0;
        self.scroll = 0;
        let message = compose_message(self.self_id, &text);
        self.remember_seen(message.id);
        self.push_message(message.clone());
        Some(message)
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

    fn push_message(&mut self, message: ChatMessage) {
        self.messages.push_back(message);
        while self.messages.len() > MAX_SCROLLBACK {
            self.messages.pop_front();
        }
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
        AppState::new(SELF_ID)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
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
        let before = app.messages.len();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        let sent = app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.input, "");
        assert_eq!(app.messages.len(), before + 1);
        assert_eq!(app.messages.back().unwrap().text, "hi");
        assert_eq!(app.messages.back().unwrap().sender, SELF_ID);
        let sent = sent.expect("enter with non-empty input returns the composed message");
        assert_eq!(sent.text, "hi");
        assert_eq!(sent.sender, SELF_ID);
    }

    #[test]
    fn empty_enter_does_not_send() {
        let mut app = app();
        let before = app.messages.len();
        let sent = app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.messages.len(), before);
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
        app.handle_net_event(NetEvent::PeerJoined(PEER_ID));
        assert_eq!(app.peers, vec![PEER_ID]);
        app.handle_net_event(NetEvent::PeerJoined(PEER_ID));
        assert_eq!(
            app.peers,
            vec![PEER_ID],
            "joining twice should not duplicate"
        );
        app.handle_net_event(NetEvent::PeerLeft(PEER_ID));
        assert!(app.peers.is_empty());
    }

    #[test]
    fn received_message_is_deduped_by_id() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hi from a peer");
        let before = app.messages.len();
        app.handle_net_event(NetEvent::Received(message.clone()));
        assert_eq!(app.messages.len(), before + 1);
        // Gossip is best-effort and can redeliver -- the same id must not
        // be shown twice.
        app.handle_net_event(NetEvent::Received(message));
        assert_eq!(app.messages.len(), before + 1);
    }
}
