//! Persisted record of joined channels and the peer addresses we've seen
//! for each, so a restart can rejoin every channel it was in before and
//! actually reconnect to peers -- not just restore "general" and rely on
//! a fresh invite ticket. This also persists each channel's `RoomSecret`
//! (see `crate::ticket`) alongside its name and peers -- see concept.md's
//! "Identity & channels" and "Room privacy" sections.
//!
//! Not to be confused with `app::Channel`, which is in-memory UI state
//! (transcript, presence) for the channels joined in the *current*
//! session -- this module only concerns itself with what's needed to
//! rejoin and reconnect on the *next* run.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use iroh::EndpointAddr;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::ticket::RoomSecret;

/// Cap on how many distinct peer addresses we remember per channel, most-
/// recently-seen first. Kept small on purpose: gossip's mesh (hyparview)
/// only needs one still-reachable bootstrap peer to rebuild the rest, so
/// this is just a hedge against the single most-recent peer having gone
/// offline by the time of the next restart.
const MAX_PEERS_PER_CHANNEL: usize = 8;

/// One channel's persisted state: its name, its room secret (see
/// `crate::ticket::RoomSecret` -- this is what actually determines its
/// gossip topic, not the name), and known peer addresses, most-recently-
/// seen first.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChannelRecord {
    name: String,
    secret: RoomSecret,
    peers: Vec<EndpointAddr>,
}

/// Local, whole-file-rewrite-on-change store of joined channels and known
/// peer addresses.
///
/// Expected to stay small (a handful of channels, a handful of addresses
/// each), so unlike storage.rs's append-only message log, it's simplest to
/// just re-encode and rewrite the whole file on every change.
pub struct ChannelRegistry {
    path: PathBuf,
    channels: Vec<ChannelRecord>,
}

impl ChannelRegistry {
    /// Loads the registry persisted at `path`, or starts empty if the file
    /// doesn't exist yet.
    ///
    /// A corrupt file is logged and treated as empty rather than failing
    /// startup -- like storage.rs's message log, this is a best-effort
    /// local cache that speeds up reconnection, not a source of truth:
    /// losing it just means falling back to a fresh invite ticket, not
    /// losing any messages. Only a genuine I/O error (e.g. permission
    /// denied) is returned as `Err`.
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

    /// Joined channel names, in the order they were first recorded.
    pub fn channel_names(&self) -> Vec<String> {
        self.channels.iter().map(|c| c.name.clone()).collect()
    }

    /// The room secret recorded for `channel`, if it's known.
    pub fn secret_for(&self, channel: &str) -> Option<RoomSecret> {
        self.channels
            .iter()
            .find(|c| c.name == channel)
            .map(|c| c.secret)
    }

    /// Known peer addresses for `channel`, most-recently-seen first. Empty
    /// if the channel isn't known, or is known but has no recorded peers
    /// yet (e.g. a channel created by name that no one has joined yet).
    pub fn bootstrap_for(&self, channel: &str) -> Vec<EndpointAddr> {
        self.channels
            .iter()
            .find(|c| c.name == channel)
            .map(|c| c.peers.clone())
            .unwrap_or_default()
    }

    /// Ensures `channel` has a registry entry recording `secret`, creating
    /// one with no known peers yet if it's new. A no-op (including no
    /// write to disk) if already recorded -- `secret` is only used the
    /// first time, since `Net` always resolves the same already-known
    /// secret for a channel it's rejoining (see `Net::start`), so there's
    /// never a conflicting one to reconcile here.
    pub fn record_channel(&mut self, channel: &str, secret: RoomSecret) -> anyhow::Result<()> {
        if self.channels.iter().any(|c| c.name == channel) {
            return Ok(());
        }
        self.channels.push(ChannelRecord {
            name: channel.to_string(),
            secret,
            peers: Vec::new(),
        });
        self.save()
    }

    /// Records `addr` as a currently-reachable peer for `channel`: moves
    /// it to the front if already known (deduped by node id, so a
    /// re-sighting doesn't create a duplicate), then evicts the oldest
    /// entry once over `MAX_PEERS_PER_CHANNEL`.
    ///
    /// A no-op (logged) if `channel` has no registry entry yet -- unlike
    /// `record_channel`, this has no `RoomSecret` to create one with, and
    /// in practice `record_channel` always runs for a channel before this
    /// is ever called for it (see main.rs).
    pub fn record_peer(&mut self, channel: &str, addr: EndpointAddr) -> anyhow::Result<()> {
        let Some(index) = self.channels.iter().position(|c| c.name == channel) else {
            warn!(
                channel,
                "learned a peer address for a not-yet-recorded channel; dropping"
            );
            return Ok(());
        };
        let peers = &mut self.channels[index].peers;
        peers.retain(|known| known.id != addr.id);
        peers.insert(0, addr);
        peers.truncate(MAX_PEERS_PER_CHANNEL);
        self.save()
    }

    /// Re-encodes and rewrites the whole registry file.
    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes =
            postcard::to_stdvec(&self.channels).context("failed to encode channel registry")?;
        fs::write(&self.path, bytes).with_context(|| {
            format!(
                "failed to write channel registry to {}",
                self.path.display()
            )
        })
    }
}

/// Decodes the registry file's contents, tolerating corruption by logging
/// and falling back to an empty registry -- see `ChannelRegistry::load`.
fn decode(bytes: &[u8]) -> Vec<ChannelRecord> {
    postcard::from_bytes(bytes).unwrap_or_else(|err| {
        warn!("dropping corrupt channel registry: {err}");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use iroh::{SecretKey, TransportAddr};

    use super::*;

    fn sample_addr() -> EndpointAddr {
        EndpointAddr::from_parts(
            SecretKey::generate().public(),
            [TransportAddr::Ip(SocketAddr::from(([127, 0, 0, 1], 4242)))],
        )
    }

    fn registry_at(dir: &tempfile::TempDir) -> ChannelRegistry {
        ChannelRegistry::load(dir.path().join("channels")).unwrap()
    }

    #[test]
    fn missing_file_is_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        assert!(registry_at(&dir).channel_names().is_empty());
    }

    #[test]
    fn corrupt_file_is_tolerated_as_an_empty_registry() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("channels"), b"not a valid registry").unwrap();
        assert!(registry_at(&dir).channel_names().is_empty());
    }

    #[test]
    fn record_channel_then_lists_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry
            .record_channel("general", RoomSecret::generate())
            .unwrap();
        assert_eq!(registry.channel_names(), vec!["general".to_string()]);
    }

    #[test]
    fn record_channel_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry
            .record_channel("general", RoomSecret::generate())
            .unwrap();
        registry
            .record_channel("general", RoomSecret::generate())
            .unwrap();
        assert_eq!(registry.channel_names(), vec!["general".to_string()]);
    }

    #[test]
    fn record_channel_keeps_the_first_secret_when_called_again() {
        // The registry is the durable source of truth for a channel's
        // secret -- a later, unrelated `record_channel` call (e.g. a
        // future `Net::start` resolving it the normal way) must never
        // silently replace it.
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        let first = RoomSecret::generate();
        registry.record_channel("general", first).unwrap();
        registry
            .record_channel("general", RoomSecret::generate())
            .unwrap();
        assert_eq!(registry.secret_for("general"), Some(first));
    }

    #[test]
    fn record_peer_then_bootstrap_for_returns_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry
            .record_channel("general", RoomSecret::generate())
            .unwrap();
        let addr = sample_addr();
        registry.record_peer("general", addr.clone()).unwrap();
        assert_eq!(registry.bootstrap_for("general"), vec![addr]);
    }

    #[test]
    fn record_peer_on_an_unrecorded_channel_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry.record_peer("nonexistent", sample_addr()).unwrap();
        assert!(registry.bootstrap_for("nonexistent").is_empty());
        assert!(registry.channel_names().is_empty());
    }

    #[test]
    fn bootstrap_for_an_unknown_channel_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(registry_at(&dir).bootstrap_for("nonexistent").is_empty());
    }

    #[test]
    fn secret_for_an_unknown_channel_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(registry_at(&dir).secret_for("nonexistent"), None);
    }

    #[test]
    fn record_peer_moves_an_already_known_peer_to_the_front_instead_of_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry
            .record_channel("general", RoomSecret::generate())
            .unwrap();
        let a = sample_addr();
        let b = sample_addr();
        registry.record_peer("general", a.clone()).unwrap();
        registry.record_peer("general", b.clone()).unwrap();
        registry.record_peer("general", a.clone()).unwrap();

        let peers = registry.bootstrap_for("general");
        assert_eq!(peers.len(), 2, "must not duplicate an already-known peer");
        assert_eq!(peers[0].id, a.id, "re-seen peer moves to the front");
        assert_eq!(peers[1].id, b.id);
    }

    #[test]
    fn record_peer_evicts_the_oldest_beyond_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry
            .record_channel("general", RoomSecret::generate())
            .unwrap();
        let addrs: Vec<EndpointAddr> = (0..MAX_PEERS_PER_CHANNEL + 3)
            .map(|_| sample_addr())
            .collect();
        for addr in &addrs {
            registry.record_peer("general", addr.clone()).unwrap();
        }

        let peers = registry.bootstrap_for("general");
        assert_eq!(peers.len(), MAX_PEERS_PER_CHANNEL);
        assert_eq!(
            peers[0].id,
            addrs.last().unwrap().id,
            "most recently recorded peer must be first"
        );
    }

    #[test]
    fn channels_are_isolated() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = registry_at(&dir);
        registry
            .record_channel("general", RoomSecret::generate())
            .unwrap();
        registry.record_peer("general", sample_addr()).unwrap();
        registry
            .record_channel("random", RoomSecret::generate())
            .unwrap();

        assert_eq!(registry.bootstrap_for("general").len(), 1);
        assert!(registry.bootstrap_for("random").is_empty());
        assert_eq!(
            registry.channel_names(),
            vec!["general".to_string(), "random".to_string()]
        );
    }

    #[test]
    fn persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channels");
        let addr = sample_addr();
        let secret = RoomSecret::generate();

        let mut registry = ChannelRegistry::load(path.clone()).unwrap();
        registry.record_channel("general", secret).unwrap();
        registry.record_peer("general", addr.clone()).unwrap();

        let reloaded = ChannelRegistry::load(path).unwrap();
        assert_eq!(reloaded.channel_names(), vec!["general".to_string()]);
        assert_eq!(reloaded.bootstrap_for("general"), vec![addr]);
        assert_eq!(reloaded.secret_for("general"), Some(secret));
    }
}
