//! Local message log persistence.
//!
//! Persists received `ChatMessage`s so scrollback survives restarts, since
//! gossip is live-broadcast only and offers no backfill -- see concept.md's
//! "Persistence & offline history" section. Landing target: build-order step
//! 5.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::PathBuf;

use anyhow::Context;
use tracing::warn;

use crate::message::ChatMessage;

/// Number of bytes used to frame each stored record's length.
const LEN_PREFIX_BYTES: usize = 4;

/// Local, append-only log of chat messages, one file per channel.
///
/// Each channel's file lives under `dir`, named by a blake3 hash of the
/// channel name rather than the name itself, so an arbitrary user-typed
/// channel name (from `/join <name>`) can never produce a path-traversal
/// or otherwise invalid filename -- the same kind of trick
/// `net::topic_for_secret` uses to turn arbitrary bytes into a safe,
/// fixed-size identifier.
///
/// Stateless and open-per-call by design: message send/receive happens at
/// human/network speed, not a hot loop, so there's no need to cache file
/// handles.
pub struct MessageStore {
    dir: PathBuf,
}

impl MessageStore {
    /// Ensures `dir` exists and returns a store rooted there.
    pub fn new(dir: PathBuf) -> anyhow::Result<Self> {
        fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;
        Ok(Self { dir })
    }

    /// Loads `channel`'s persisted history, oldest message first. Returns
    /// an empty list if the channel has no history yet.
    ///
    /// A corrupt or truncated trailing record (e.g. the process was killed
    /// mid-write) is logged and the messages recorded before it are still
    /// returned, rather than failing outright -- this log is a best-effort
    /// local cache, not the source of truth (concept.md's "Persistence &
    /// offline history" section), so availability wins over strictness
    /// here. Only a genuine I/O error (e.g. permission denied) is returned
    /// as `Err`.
    pub fn load(&self, channel: &str) -> anyhow::Result<Vec<ChatMessage>> {
        let path = self.path_for(channel);
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(decode_all(&bytes))
    }

    /// Appends `message` to `channel`'s log.
    pub fn append(&self, channel: &str, message: &ChatMessage) -> anyhow::Result<()> {
        let path = self.path_for(channel);
        let payload = postcard::to_stdvec(message).context("failed to encode chat message")?;
        let len = u32::try_from(payload.len()).context("chat message too large to persist")?;

        let mut record = Vec::with_capacity(LEN_PREFIX_BYTES + payload.len());
        record.extend_from_slice(&len.to_le_bytes());
        record.extend_from_slice(&payload);

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open {}", path.display()))?;
        file.write_all(&record)
            .with_context(|| format!("failed to write to {}", path.display()))?;
        Ok(())
    }

    /// Derives `channel`'s log file path from a blake3 hash of its name.
    fn path_for(&self, channel: &str) -> PathBuf {
        self.dir
            .join(format!("{}.log", blake3::hash(channel.as_bytes()).to_hex()))
    }
}

/// Decodes a sequence of `[u32 LE length][postcard bytes]` records,
/// stopping (and warning) at the first truncated or malformed record
/// instead of failing outright -- see `MessageStore::load`.
fn decode_all(mut bytes: &[u8]) -> Vec<ChatMessage> {
    let mut messages = Vec::new();
    while !bytes.is_empty() {
        let Some((len_bytes, rest)) = bytes.split_at_checked(LEN_PREFIX_BYTES) else {
            warn!("truncated length prefix in message log, ignoring remainder");
            break;
        };
        let len = u32::from_le_bytes(len_bytes.try_into().expect("length checked above")) as usize;
        let Some((payload, rest)) = rest.split_at_checked(len) else {
            warn!("truncated message record in message log, ignoring remainder");
            break;
        };
        match postcard::from_bytes::<ChatMessage>(payload) {
            Ok(message) => messages.push(message),
            Err(err) => {
                warn!("dropping malformed message log entry: {err}");
                break;
            }
        }
        bytes = rest;
    }
    messages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_message(id: u64, text: &str) -> ChatMessage {
        ChatMessage {
            v: 1,
            id,
            sender: [1; 32],
            ts_unix_ms: 0,
            text: text.to_string(),
        }
    }

    #[test]
    fn load_returns_empty_when_no_history_exists() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();
        assert_eq!(store.load("general").unwrap(), Vec::new());
    }

    #[test]
    fn append_then_load_round_trips_messages_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();
        let messages = vec![sample_message(1, "hi"), sample_message(2, "there")];
        for message in &messages {
            store.append("general", message).unwrap();
        }
        assert_eq!(store.load("general").unwrap(), messages);
    }

    #[test]
    fn channels_are_stored_in_isolated_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();
        store
            .append("general", &sample_message(1, "in general"))
            .unwrap();
        store
            .append("random", &sample_message(2, "in random"))
            .unwrap();

        assert_eq!(
            store.load("general").unwrap(),
            vec![sample_message(1, "in general")]
        );
        assert_eq!(
            store.load("random").unwrap(),
            vec![sample_message(2, "in random")]
        );
    }

    #[test]
    fn channel_names_with_path_like_characters_stay_within_the_store_dir() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();
        store
            .append("../../etc/passwd", &sample_message(1, "hi"))
            .unwrap();

        assert_eq!(
            store.load("../../etc/passwd").unwrap(),
            vec![sample_message(1, "hi")]
        );
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(
            entries.len(),
            1,
            "the record must land inside dir, not escape it"
        );
    }

    #[test]
    fn recovers_messages_before_a_truncated_length_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();
        let message = sample_message(1, "hi");
        store.append("general", &message).unwrap();

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(store.path_for("general"))
            .unwrap();
        file.write_all(&[0xFF, 0xFF]).unwrap(); // fewer than LEN_PREFIX_BYTES bytes

        assert_eq!(store.load("general").unwrap(), vec![message]);
    }

    #[test]
    fn recovers_messages_before_a_truncated_payload() {
        let dir = tempfile::tempdir().unwrap();
        let store = MessageStore::new(dir.path().to_path_buf()).unwrap();
        let message = sample_message(1, "hi");
        store.append("general", &message).unwrap();

        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(store.path_for("general"))
            .unwrap();
        // Claims a 100-byte payload follows but provides none.
        file.write_all(&100u32.to_le_bytes()).unwrap();

        assert_eq!(store.load("general").unwrap(), vec![message]);
    }
}
