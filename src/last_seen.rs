//! Persisted last-seen timestamps for peers, scoped to individual channels.
//!
//! This is local display state, not network membership: gossip still decides
//! whether a peer is online for the current session. The small store mirrors
//! `contacts.rs` by rewriting one postcard file whenever an observation
//! changes.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChannelRecord {
    name: String,
    peers: Vec<PeerRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PeerRecord {
    id: [u8; 32],
    seen_unix_ms: u64,
}

pub struct LastSeen {
    path: PathBuf,
    channels: Vec<ChannelRecord>,
}

impl LastSeen {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let channels = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, channels })
    }

    pub fn snapshot(&self) -> HashMap<String, HashMap<[u8; 32], u64>> {
        self.channels
            .iter()
            .map(|channel| {
                (
                    channel.name.clone(),
                    channel
                        .peers
                        .iter()
                        .map(|peer| (peer.id, peer.seen_unix_ms))
                        .collect(),
                )
            })
            .collect()
    }

    pub fn record(&mut self, channel: &str, id: [u8; 32], seen_unix_ms: u64) -> anyhow::Result<()> {
        let record = match self.channels.iter_mut().find(|known| known.name == channel) {
            Some(record) => record,
            None => {
                self.channels.push(ChannelRecord {
                    name: channel.to_string(),
                    peers: Vec::new(),
                });
                self.channels
                    .last_mut()
                    .expect("just pushed channel record")
            }
        };
        match record.peers.iter_mut().find(|peer| peer.id == id) {
            Some(peer) => peer.seen_unix_ms = seen_unix_ms,
            None => record.peers.push(PeerRecord { id, seen_unix_ms }),
        }
        self.save()
    }

    pub fn forget_channel(&mut self, channel: &str) -> anyhow::Result<()> {
        let before = self.channels.len();
        self.channels.retain(|known| known.name != channel);
        if self.channels.len() == before {
            return Ok(());
        }
        self.save()
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes =
            postcard::to_stdvec(&self.channels).context("failed to encode last-seen peers")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }
}

fn decode(bytes: &[u8]) -> Vec<ChannelRecord> {
    postcard::from_bytes(bytes).unwrap_or_else(|err| {
        warn!("dropping corrupt last-seen peer data: {err}");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: [u8; 32] = [1; 32];
    const BOB: [u8; 32] = [2; 32];

    fn store_at(dir: &tempfile::TempDir) -> LastSeen {
        LastSeen::load(dir.path().join("last_seen")).unwrap()
    }

    #[test]
    fn missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(store_at(&dir).snapshot().is_empty());
    }

    #[test]
    fn corrupt_file_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("last_seen"), b"not postcard").unwrap();
        assert!(store_at(&dir).snapshot().is_empty());
    }

    #[test]
    fn records_are_scoped_to_their_channel() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_at(&dir);
        store.record("general", ALICE, 10).unwrap();
        store.record("random", BOB, 20).unwrap();

        assert_eq!(
            store.snapshot(),
            HashMap::from([
                ("general".to_string(), HashMap::from([(ALICE, 10)])),
                ("random".to_string(), HashMap::from([(BOB, 20)])),
            ])
        );
    }

    #[test]
    fn recording_the_same_peer_refreshes_its_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_at(&dir);
        store.record("general", ALICE, 10).unwrap();
        store.record("general", ALICE, 20).unwrap();

        assert_eq!(
            store.snapshot(),
            HashMap::from([("general".to_string(), HashMap::from([(ALICE, 20)]))])
        );
    }

    #[test]
    fn records_persist_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("last_seen");
        let mut store = LastSeen::load(path.clone()).unwrap();
        store.record("general", ALICE, 10).unwrap();

        assert_eq!(
            LastSeen::load(path).unwrap().snapshot(),
            HashMap::from([("general".to_string(), HashMap::from([(ALICE, 10)]))])
        );
    }

    #[test]
    fn forgetting_a_channel_keeps_other_channels() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_at(&dir);
        store.record("general", ALICE, 10).unwrap();
        store.record("random", BOB, 20).unwrap();
        store.forget_channel("general").unwrap();

        assert_eq!(
            store.snapshot(),
            HashMap::from([("random".to_string(), HashMap::from([(BOB, 20)]))])
        );
    }
}
