//! Persisted mapping from a peer's endpoint id to the local name of the
//! private, auto-provisioned channel used to `/msg` them -- see
//! features.md's "Direct 1:1 DMs" and `app::AppState::run_msg`. Lets both
//! the person who ran `/msg` and the person who accepted their invite
//! converge on reusing the same channel for that pair, instead of each
//! side accumulating its own separate room -- see `ticket::ChannelTicket`'s
//! `dm` marker, which is what tells the recipient's side to record an
//! entry here too.
//!
//! Mirrors `channel_registry.rs`'s persisted-postcard-file pattern: small,
//! infrequently-changed data, so it's simplest to just re-encode and
//! rewrite the whole file on every change. Local only, like
//! `contacts.rs` -- never sent over the wire.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// One peer's DM channel association: the endpoint id it's for, and the
/// local channel name used to talk to them.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DmRecord {
    peer: [u8; 32],
    channel: String,
}

/// Local, whole-file-rewrite-on-change store of peer <-> DM channel
/// associations.
///
/// Expected to stay small (a handful of DMs), so like `channel_registry.rs`
/// and `contacts.rs`, it's simplest to just re-encode and rewrite the whole
/// file on every change.
pub struct DmRegistry {
    path: PathBuf,
    entries: Vec<DmRecord>,
}

impl DmRegistry {
    /// Loads the registry persisted at `path`, or starts empty if the file
    /// doesn't exist yet.
    ///
    /// A corrupt file is logged and treated as empty rather than failing
    /// startup -- like `channel_registry.rs`, losing it just means `/msg`
    /// creates a fresh DM channel next time instead of reusing an old one,
    /// not any real data loss. Only a genuine I/O error (e.g. permission
    /// denied) is returned as `Err`.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let entries = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, entries })
    }

    /// All known peer -> DM channel associations, seeded into `AppState`
    /// once at startup (see `AppState::load_dm_registry`).
    pub fn all(&self) -> HashMap<[u8; 32], String> {
        self.entries
            .iter()
            .map(|e| (e.peer, e.channel.clone()))
            .collect()
    }

    /// Whether `channel` is currently recorded as some peer's DM channel --
    /// consulted whenever a ticket is built (see `net::Net::ticket_for`),
    /// so a DM's ticket always carries the `dm` marker, even on a
    /// regenerated `/invite`, not just the one built when it was created.
    pub fn is_dm_channel(&self, channel: &str) -> bool {
        self.entries.iter().any(|e| e.channel == channel)
    }

    /// Records (or overwrites) the DM channel used for `peer` -- called
    /// once a `/msg`-initiated join actually succeeds, by both the person
    /// who started it and, via the ticket's `dm` marker, the person who
    /// accepted it (see `net::NetEvent::DmJoined`).
    pub fn record(&mut self, peer: [u8; 32], channel: String) -> anyhow::Result<()> {
        match self.entries.iter_mut().find(|e| e.peer == peer) {
            Some(existing) => existing.channel = channel,
            None => self.entries.push(DmRecord { peer, channel }),
        }
        self.save()
    }

    /// Forgets whichever entry points at `channel`, if any -- called
    /// alongside `channel_registry::ChannelRegistry::forget_channel` when
    /// that channel is `/leave`d, so a later `/msg` to the same peer
    /// starts fresh rather than trying to reuse a channel that no longer
    /// exists. A no-op (including no write to disk) if no entry points at
    /// `channel`.
    pub fn forget_channel(&mut self, channel: &str) -> anyhow::Result<()> {
        let before = self.entries.len();
        self.entries.retain(|e| e.channel != channel);
        if self.entries.len() == before {
            return Ok(());
        }
        self.save()
    }

    /// Re-encodes and rewrites the whole registry file.
    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes = postcard::to_stdvec(&self.entries).context("failed to encode DM registry")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write DM registry to {}", self.path.display()))
    }
}

/// Decodes the registry file's contents, tolerating corruption by logging
/// and falling back to an empty list -- see `DmRegistry::load`.
fn decode(bytes: &[u8]) -> Vec<DmRecord> {
    postcard::from_bytes(bytes).unwrap_or_else(|err| {
        warn!("dropping corrupt DM registry: {err}");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: [u8; 32] = [1; 32];
    const BOB: [u8; 32] = [2; 32];

    fn registry_at(dir: &tempfile::TempDir) -> DmRegistry {
        DmRegistry::load(dir.path().join("dms")).unwrap()
    }

    #[test]
    fn missing_file_is_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        assert!(registry_at(&dir).all().is_empty());
    }

    #[test]
    fn corrupt_file_is_tolerated_as_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("dms"), b"not a valid registry").unwrap();
        assert!(registry_at(&dir).all().is_empty());
    }

    #[test]
    fn record_then_all_contains_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE, "dm-alice".to_string()).unwrap();
        assert_eq!(
            registry.all(),
            HashMap::from([(ALICE, "dm-alice".to_string())])
        );
    }

    #[test]
    fn record_again_overwrites_the_existing_channel() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE, "dm-alice".to_string()).unwrap();
        registry.record(ALICE, "dm-alice-2".to_string()).unwrap();
        assert_eq!(
            registry.all(),
            HashMap::from([(ALICE, "dm-alice-2".to_string())]),
            "re-recording the same peer must overwrite, not duplicate"
        );
    }

    #[test]
    fn entries_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE, "dm-alice".to_string()).unwrap();
        registry.record(BOB, "dm-bob".to_string()).unwrap();
        assert_eq!(
            registry.all(),
            HashMap::from([
                (ALICE, "dm-alice".to_string()),
                (BOB, "dm-bob".to_string())
            ])
        );
    }

    #[test]
    fn is_dm_channel_reflects_recorded_entries() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE, "dm-alice".to_string()).unwrap();
        assert!(registry.is_dm_channel("dm-alice"));
        assert!(!registry.is_dm_channel("general"));
    }

    #[test]
    fn forget_channel_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE, "dm-alice".to_string()).unwrap();
        registry.forget_channel("dm-alice").unwrap();
        assert!(registry.all().is_empty());
        assert!(!registry.is_dm_channel("dm-alice"));
    }

    #[test]
    fn forget_channel_is_a_no_op_for_an_unknown_channel() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE, "dm-alice".to_string()).unwrap();
        registry.forget_channel("dm-nonexistent").unwrap();
        assert_eq!(
            registry.all(),
            HashMap::from([(ALICE, "dm-alice".to_string())])
        );
    }

    #[test]
    fn forget_channel_only_removes_the_named_channel() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE, "dm-alice".to_string()).unwrap();
        registry.record(BOB, "dm-bob".to_string()).unwrap();
        registry.forget_channel("dm-alice").unwrap();
        assert_eq!(registry.all(), HashMap::from([(BOB, "dm-bob".to_string())]));
    }

    #[test]
    fn persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dms");

        let mut registry = DmRegistry::load(path.clone()).unwrap();
        registry.record(ALICE, "dm-alice".to_string()).unwrap();

        let reloaded = DmRegistry::load(path).unwrap();
        assert_eq!(
            reloaded.all(),
            HashMap::from([(ALICE, "dm-alice".to_string())])
        );
    }

    #[test]
    fn forget_channel_persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dms");

        let mut registry = DmRegistry::load(path.clone()).unwrap();
        registry.record(ALICE, "dm-alice".to_string()).unwrap();
        registry.forget_channel("dm-alice").unwrap();

        let reloaded = DmRegistry::load(path).unwrap();
        assert!(reloaded.all().is_empty());
    }
}
