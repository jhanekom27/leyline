//! Owns all UI state and the pure logic that mutates it.
//!
//! Per concept.md's "one owner of UI state" principle: nothing here does
//! network or file I/O. `handle_key` is a pure function of the current state
//! and a keypress, safe to call directly from the render loop.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::message::ChatMessage;

/// Cap on in-memory scrollback, per concept.md's guidance on bounding
/// render/memory cost in a long-lived session.
const MAX_SCROLLBACK: usize = 500;

/// Fake sender ids used to seed the UI before identity.rs exists.
const SELF_SENDER: [u8; 32] = [0; 32];
const ALICE_SENDER: [u8; 32] = [1; 32];
const BOB_SENDER: [u8; 32] = [2; 32];

/// All state needed to render the TUI and respond to input.
pub struct AppState {
    pub messages: VecDeque<ChatMessage>,
    pub peers: Vec<String>,
    pub input: String,
    pub scroll: usize,
    pub should_quit: bool,
}

impl AppState {
    /// Builds the initial state, seeded with a few fake messages/peers so
    /// the layout can be visually validated without real networking.
    pub fn new() -> Self {
        let mut messages = VecDeque::new();
        messages.push_back(fake_message(ALICE_SENDER, "hey, anyone around?"));
        messages.push_back(fake_message(BOB_SENDER, "just got here"));
        messages.push_back(fake_message(SELF_SENDER, "o/"));

        Self {
            messages,
            peers: vec!["alice".to_string(), "bob".to_string()],
            input: String::new(),
            scroll: 0,
            should_quit: false,
        }
    }

    /// Pure key handling: no I/O, no network. Safe to call on every keypress.
    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
            }
            KeyCode::Enter => self.submit_input(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Up => self.scroll = self.scroll.saturating_add(1),
            KeyCode::Down => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
    }

    fn submit_input(&mut self) {
        if self.input.trim().is_empty() {
            return;
        }
        let text = std::mem::take(&mut self.input);
        self.push_message(fake_message(SELF_SENDER, &text));
        self.scroll = 0;
    }

    fn push_message(&mut self, message: ChatMessage) {
        self.messages.push_back(message);
        while self.messages.len() > MAX_SCROLLBACK {
            self.messages.pop_front();
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

/// Maps a known fake sender id to a display name, falling back to a
/// shortened hex id -- the same fallback real, unnamed peers will use later.
pub fn display_name(sender: &[u8; 32]) -> String {
    if *sender == SELF_SENDER {
        "you".to_string()
    } else if *sender == ALICE_SENDER {
        "alice".to_string()
    } else if *sender == BOB_SENDER {
        "bob".to_string()
    } else {
        hex_prefix(sender)
    }
}

fn hex_prefix(bytes: &[u8; 32]) -> String {
    bytes[..4].iter().map(|b| format!("{b:02x}")).collect()
}

fn next_id() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn fake_message(sender: [u8; 32], text: &str) -> ChatMessage {
    ChatMessage {
        v: 1,
        id: next_id(),
        sender,
        ts_unix_ms: now_unix_ms(),
        text: text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn typing_composes_input() {
        let mut app = AppState::new();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        assert_eq!(app.input, "hi");
    }

    #[test]
    fn backspace_edits_input() {
        let mut app = AppState::new();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.input, "h");
    }

    #[test]
    fn enter_sends_and_clears_input() {
        let mut app = AppState::new();
        let before = app.messages.len();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.input, "");
        assert_eq!(app.messages.len(), before + 1);
        assert_eq!(app.messages.back().unwrap().text, "hi");
        assert_eq!(app.messages.back().unwrap().sender, SELF_SENDER);
    }

    #[test]
    fn empty_enter_does_not_send() {
        let mut app = AppState::new();
        let before = app.messages.len();
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.messages.len(), before);
    }

    #[test]
    fn esc_quits() {
        let mut app = AppState::new();
        app.handle_key(key(KeyCode::Esc));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = AppState::new();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn display_name_maps_known_fake_senders() {
        assert_eq!(display_name(&SELF_SENDER), "you");
        assert_eq!(display_name(&ALICE_SENDER), "alice");
        assert_eq!(display_name(&BOB_SENDER), "bob");
    }
}
