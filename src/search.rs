//! Local chat history search -- see features.md's "Local search" idea.
//!
//! Matching is one shared definition (`matches`), used both for the
//! instant scan over a channel's loaded scrollback (`app::Channel`) and
//! the background scan over its full on-disk log (`run_on_disk_scan`,
//! spawned via `tokio::task::spawn_blocking` from main.rs so a large log
//! never blocks the render loop -- see concept.md's "snappy async event
//! loop").

use tracing::warn;

use crate::message::ChatMessage;
use crate::storage::MessageStore;

/// A channel's active `/search <term>` results -- see `app::Channel::search`.
///
/// A point-in-time snapshot, not a live filter: re-run `/search <term>` to
/// pick up messages sent or received while it's active.
pub struct SearchResults {
    /// As typed after `/search`, shown back in the pane title
    /// (`ui::render_messages`) and used to detect a stale `SearchOutcome`
    /// from a superseded search (`app::AppState::apply_search_outcome`).
    pub term: String,
    /// Matches found so far, oldest first. Seeded from the loaded
    /// scrollback, then extended once the background on-disk scan
    /// completes.
    pub messages: Vec<ChatMessage>,
    /// `true` until the on-disk scan finishes, so the UI can show a
    /// "searching..." indicator.
    pub pending: bool,
}

/// Sent back over main.rs's dedicated search channel once a background
/// on-disk scan (`run_on_disk_scan`) finishes.
pub struct SearchOutcome {
    pub channel: String,
    pub term: String,
    /// Matches found in the full on-disk log, oldest first. Empty (rather
    /// than the search failing outright) if the log couldn't be read --
    /// see `run_on_disk_scan`.
    pub messages: Vec<ChatMessage>,
}

/// Case-insensitive substring match against a message's text -- the one
/// definition of "matches" shared by the in-memory and on-disk scans.
/// `term_lower` must already be lowercased -- callers scanning many
/// messages against the same term should lowercase it once, not per call.
pub fn matches(text: &str, term_lower: &str) -> bool {
    text.to_lowercase().contains(term_lower)
}

/// Whether `message` matches `term_lower` for the purposes of `/search`:
/// its text, or -- since a file share's `text` is typically empty -- its
/// attachment's filename, if it has one. The one definition of "does this
/// message match", shared by `app::Channel::search_loaded` (the in-memory
/// scan) and `run_on_disk_scan` below.
pub fn message_matches(message: &ChatMessage, term_lower: &str) -> bool {
    matches(&message.text, term_lower)
        || message
            .attachment
            .as_ref()
            .is_some_and(|attachment| matches(&attachment.filename, term_lower))
}

/// Scans `channel`'s full on-disk log (`storage::MessageStore::load`) for
/// messages matching `term`, oldest first. Synchronous -- the caller
/// (main.rs) is expected to run this inside a `tokio::task::spawn_blocking`,
/// since a large log can't be assumed cheap enough for the async event
/// loop.
///
/// A read failure is logged and reported as zero matches rather than
/// failing the search outright -- storage.rs's log is a best-effort local
/// cache, not a source of truth, and the caller already has whatever the
/// in-memory scan found regardless.
pub fn run_on_disk_scan(store: &MessageStore, channel: &str, term: &str) -> SearchOutcome {
    let term_lower = term.to_lowercase();
    let messages = match store.load(channel) {
        Ok(all) => all
            .into_iter()
            .filter(|message| message_matches(message, &term_lower))
            .collect(),
        Err(err) => {
            warn!(%channel, "search failed to read on-disk history: {err}");
            Vec::new()
        }
    };
    SearchOutcome {
        channel: channel.to_string(),
        term: term.to_string(),
        messages,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(id: u64, text: &str) -> ChatMessage {
        ChatMessage {
            v: 1,
            id,
            sender: [1; 32],
            ts_unix_ms: id,
            text: text.to_string(),
            attachment: None,
        }
    }

    fn file_message(id: u64, filename: &str) -> ChatMessage {
        ChatMessage {
            v: 2,
            id,
            sender: [1; 32],
            ts_unix_ms: id,
            text: String::new(),
            attachment: Some(crate::message::FileAttachment {
                filename: filename.to_string(),
                size: 0,
                hash: iroh_blobs::Hash::new(filename.as_bytes()),
            }),
        }
    }

    #[test]
    fn matches_is_case_insensitive_with_respect_to_the_haystack() {
        // `term_lower` is documented as already-lowercased (callers
        // lowercase it once up front), so only `text`'s casing varies here.
        assert!(matches("Hello World", "world"));
        assert!(matches("HELLO WORLD", "hello"));
        assert!(!matches("Hello World", "xyz"));
    }

    #[test]
    fn matches_finds_substrings_not_just_whole_words() {
        assert!(matches("searching", "arch"));
    }

    #[test]
    fn run_on_disk_scan_returns_empty_when_no_history_exists() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();

        let outcome = run_on_disk_scan(&store, "general", "term");

        assert!(outcome.messages.is_empty());
        assert_eq!(outcome.channel, "general");
        assert_eq!(outcome.term, "term");
    }

    #[test]
    fn run_on_disk_scan_finds_matches_and_filters_non_matches() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();
        store.append("general", &message(1, "hello world")).unwrap();
        store.append("general", &message(2, "goodbye")).unwrap();
        store.append("general", &message(3, "WORLD tour")).unwrap();

        let outcome = run_on_disk_scan(&store, "general", "world");

        assert_eq!(
            outcome.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 3]
        );
    }

    #[test]
    fn run_on_disk_scan_is_isolated_per_channel() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();
        store.append("general", &message(1, "match me")).unwrap();
        store.append("random", &message(2, "match me too")).unwrap();

        let outcome = run_on_disk_scan(&store, "general", "match");

        assert_eq!(outcome.messages.len(), 1);
        assert_eq!(outcome.messages[0].id, 1);
    }

    #[test]
    fn message_matches_finds_a_captionless_files_name() {
        let shared = file_message(1, "vacation-photo.png");
        assert!(message_matches(&shared, "vacation"));
        assert!(!message_matches(&shared, "nonexistent"));
    }

    #[test]
    fn run_on_disk_scan_finds_a_shared_files_name() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();
        store
            .append("general", &file_message(1, "vacation-photo.png"))
            .unwrap();
        store.append("general", &message(2, "unrelated")).unwrap();

        let outcome = run_on_disk_scan(&store, "general", "vacation");

        assert_eq!(outcome.messages.len(), 1);
        assert_eq!(outcome.messages[0].id, 1);
    }
}
