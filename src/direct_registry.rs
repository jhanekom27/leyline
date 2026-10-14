//! Persisted set of peers we have (or had) a point-to-point conversation
//! with -- see `direct.rs` and features/LEY-12's "True direct 1:1
//! messaging over a point-to-point stream". Unlike `dm_registry.rs`, no
//! channel name needs storing alongside each peer: a point-to-point
//! channel's name is always `direct::channel_name(&peer)`, a deterministic
//! function of the peer id alone, so the id is the only durable state.
//!
//! Mirrors `dm_registry.rs`'s persisted-postcard-file pattern: small,
//! infrequently-changed data, so it's simplest to just re-encode and
//! rewrite the whole file on every change.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use iroh_docs::Capability;
use serde::{Deserialize, Serialize};
use tracing::warn;
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirectRecord {
    peer: [u8; 32],
    docs: Option<Capability>,
}

/// Local, whole-file-rewrite-on-change store of known point-to-point peer
/// ids.
///
/// Expected to stay small (a handful of point-to-point conversations), so
/// like `dm_registry.rs` and `channel_registry.rs`, it's simplest to just
/// re-encode and rewrite the whole file on every change.
pub struct DirectRegistry {
    path: PathBuf,
    peers: Vec<DirectRecord>,
}

impl DirectRegistry {
    /// Loads the registry persisted at `path`, or starts empty if the file
    /// doesn't exist yet.
    ///
    /// A corrupt file is logged and treated as empty rather than failing
    /// startup -- like `dm_registry.rs`, losing it just means a
    /// point-to-point conversation's tab won't be restored automatically
    /// next time (though its message history on disk is untouched), not
    /// any real data loss. Only a genuine I/O error (e.g. permission
    /// denied) is returned as `Err`.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let peers = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, peers })
    }

    /// Every known point-to-point peer, seeded into the startup restore
    /// loop (see main.rs) so each one's tab, history, and connection
    /// attempt come back automatically, mirroring how `channel_registry.rs`
    /// rejoins gossip channels.
    pub fn all(&self) -> Vec<[u8; 32]> {
        self.peers.iter().map(|record| record.peer).collect()
    }

    /// The shared docs capability last negotiated with `peer`, if any.
    pub fn docs_for(&self, peer: [u8; 32]) -> Option<Capability> {
        self.peers
            .iter()
            .find(|record| record.peer == peer)
            .and_then(|record| record.docs.clone())
    }

    /// Records `peer` as having a point-to-point conversation, if not
    /// already known -- called whenever a point-to-point tab is newly
    /// created this session (see `direct.rs`'s `NetEvent::DirectJoined`
    /// and `InputAction::OpenDirect` handling in main.rs). A no-op
    /// (including no write to disk) if already recorded.
    pub fn record(&mut self, peer: [u8; 32]) -> anyhow::Result<()> {
        if self.peers.iter().any(|record| record.peer == peer) {
            return Ok(());
        }
        self.peers.push(DirectRecord { peer, docs: None });
        self.save()
    }
    /// Stores the shared docs capability after it is created or received
    /// through the authenticated direct protocol.
    pub fn record_docs(&mut self, peer: [u8; 32], docs: Capability) -> anyhow::Result<()> {
        let Some(record) = self.peers.iter_mut().find(|record| record.peer == peer) else {
            anyhow::bail!("cannot persist docs capability for unknown direct peer");
        };
        if record
            .docs
            .as_ref()
            .is_some_and(|known| known.id() == docs.id())
        {
            return Ok(());
        }
        record.docs = Some(docs);
        self.save()
    }

    /// Forgets `peer` -- called alongside `InputAction::Leave` for a
    /// point-to-point channel, so a future restart no longer tries to
    /// restore or reconnect to it. A no-op (including no write to disk) if
    /// `peer` isn't recorded.
    pub fn forget(&mut self, peer: [u8; 32]) -> anyhow::Result<()> {
        let before = self.peers.len();
        self.peers.retain(|known| known.peer != peer);
        if self.peers.len() == before {
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
        let bytes = postcard::to_stdvec(&self.peers).context("failed to encode direct registry")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write direct registry to {}", self.path.display()))
    }
}

/// Decodes the registry file's contents, tolerating corruption by logging
/// and falling back to an empty list -- see `DirectRegistry::load`.
fn decode(bytes: &[u8]) -> Vec<DirectRecord> {
    if let Ok(records) = postcard::from_bytes(bytes) {
        return records;
    }
    if let Ok(peers) = postcard::from_bytes::<Vec<[u8; 32]>>(bytes) {
        return peers
            .into_iter()
            .map(|peer| DirectRecord { peer, docs: None })
            .collect();
    }
    warn!("dropping corrupt direct-peer registry");
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: [u8; 32] = [1; 32];
    const BOB: [u8; 32] = [2; 32];

    fn registry_at(dir: &tempfile::TempDir) -> DirectRegistry {
        DirectRegistry::load(dir.path().join("direct_peers")).unwrap()
    }

    #[test]
    fn missing_file_is_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        assert!(registry_at(&dir).all().is_empty());
    }

    #[test]
    fn corrupt_file_is_tolerated_as_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("direct_peers"), b"not a valid registry").unwrap();
        assert!(registry_at(&dir).all().is_empty());
    }
    #[test]
    fn peer_only_records_migrate_without_a_docs_capability() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("direct_peers");
        fs::write(&path, postcard::to_stdvec(&vec![ALICE]).unwrap()).unwrap();

        let registry = DirectRegistry::load(path).unwrap();

        assert_eq!(registry.all(), vec![ALICE]);
        assert!(registry.docs_for(ALICE).is_none());
    }

    #[test]
    fn record_then_all_contains_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE).unwrap();
        assert_eq!(registry.all(), vec![ALICE]);
    }

    #[test]
    fn record_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE).unwrap();
        registry.record(ALICE).unwrap();
        assert_eq!(registry.all(), vec![ALICE], "must not duplicate an entry");
    }

    #[test]
    fn entries_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE).unwrap();
        registry.record(BOB).unwrap();
        assert_eq!(registry.all(), vec![ALICE, BOB]);
    }

    #[test]
    fn forget_removes_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE).unwrap();
        registry.forget(ALICE).unwrap();
        assert!(registry.all().is_empty());
    }

    #[test]
    fn forget_is_a_no_op_for_an_unknown_peer() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE).unwrap();
        registry.forget(BOB).unwrap();
        assert_eq!(registry.all(), vec![ALICE]);
    }

    #[test]
    fn forget_only_removes_the_named_peer() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record(ALICE).unwrap();
        registry.record(BOB).unwrap();
        registry.forget(ALICE).unwrap();
        assert_eq!(registry.all(), vec![BOB]);
    }

    #[test]
    fn persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("direct_peers");

        let mut registry = DirectRegistry::load(path.clone()).unwrap();
        registry.record(ALICE).unwrap();

        let reloaded = DirectRegistry::load(path).unwrap();
        assert_eq!(reloaded.all(), vec![ALICE]);
    }

    #[test]
    fn forget_persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("direct_peers");

        let mut registry = DirectRegistry::load(path.clone()).unwrap();
        registry.record(ALICE).unwrap();
        registry.forget(ALICE).unwrap();

        let reloaded = DirectRegistry::load(path).unwrap();
        assert!(reloaded.all().is_empty());
    }

    /// End-to-end evidence for LEY-12's AC #3 ("message history for the
    /// conversation is available after a restart"): mirrors exactly what
    /// main.rs's startup restore loop does -- looks up a point-to-point
    /// peer's deterministic channel name (`direct::channel_name`) and
    /// reloads its persisted history -- from the same on-disk files a
    /// previous "session" wrote, using the real `DirectRegistry` and
    /// `storage::MessageStore` types together rather than trusting each
    /// in isolation.
    #[test]
    fn a_restart_restores_both_the_peer_list_and_message_history() {
        let dir = tempfile::tempdir().unwrap();
        let registry_path = dir.path().join("direct_peers");
        let store_dir = dir.path().join("messages");
        let peer = [3; 32];
        let channel = crate::direct::channel_name(&peer);

        // "Session 1": a /dm conversation happens and a message is sent.
        {
            let mut registry = DirectRegistry::load(registry_path.clone()).unwrap();
            registry.record(peer).unwrap();
            let store = crate::storage::MessageStore::new(store_dir.clone()).unwrap();
            let message = crate::message::ChatMessage {
                v: 3,
                id: 1,
                sender: peer,
                ts_unix_ms: 1_000,
                text: "see you tomorrow".to_string(),
                attachment: None,
                reply_to: None,
            };
            store.append(&channel, &message).unwrap();
        }

        // "Session 2": a fresh process restarts, loading the same paths.
        let registry = DirectRegistry::load(registry_path).unwrap();
        let store = crate::storage::MessageStore::new(store_dir).unwrap();

        assert_eq!(registry.all(), vec![peer], "the peer must still be known");
        let history = store.load(&channel).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].text, "see you tomorrow");
    }
}
