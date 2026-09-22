//! iroh-blobs-backed history manifests, for backfilling peers that missed
//! messages while offline.
//!
//! Gossip (net.rs) is live-broadcast only, so a peer that was offline (or
//! joins a channel fresh) never receives messages sent before it was
//! listening -- see concept.md's "Persistence & history backfill" section.
//! This module is the content-addressed half of the fix: every known
//! `ChatMessage` is stored as its own blob, and a per-channel `HashSeq`
//! manifest lists them in a canonical order, so its hash can stand in for
//! "the sender's current view of this channel's history". net.rs
//! broadcasts that hash (see `crate::message::HistoryAnnounce`) whenever a
//! channel gains a gossip neighbor, and uses `fetch` here to pull anything a
//! peer is missing.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use iroh::{Endpoint, EndpointId};
use iroh_blobs::hashseq::HashSeq;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, BlobsProtocol, Hash, HashAndFormat};
use tracing::warn;

use crate::message::ChatMessage;

/// A channel's canonical, sorted view of what's gone into its manifest so
/// far, plus the manifest's current root hash.
///
/// Kept in memory (never persisted separately -- see `BackfillStore`) so
/// `record_messages` can extend it cheaply: adding one more message only
/// means hashing that message and rebuilding the (small) `HashSeq` from the
/// updated list, not re-touching every earlier message's blob.
struct ChannelManifest {
    /// Sorted by `(ts_unix_ms, id)` -- the canonical order the manifest's
    /// `HashSeq` is built from, so two peers holding the same set of
    /// messages always compute the same root regardless of when each of
    /// them learned about each one.
    entries: Vec<(u64, u64, Hash)>,
    root: Hash,
}

/// Owns the local content-addressed blob store backing history backfill, and
/// tracks each joined channel's current manifest (see `ChannelManifest`) in
/// memory, never persisted separately.
#[derive(Clone)]
pub struct BackfillStore {
    store: FsStore,
    channels: Arc<Mutex<HashMap<String, ChannelManifest>>>,
}

impl BackfillStore {
    /// Opens (or creates) the blob store rooted at `dir`.
    pub async fn new(dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let dir = dir.as_ref();
        let store = FsStore::load(dir)
            .await
            .map_err(|err| anyhow::anyhow!("failed to open {}: {err}", dir.display()))?;
        Ok(Self {
            store,
            channels: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    /// Builds an iroh-blobs protocol handler for this store, so net.rs can
    /// register it on the shared `Router` alongside gossip and serve blobs
    /// to peers that fetch from us.
    pub fn protocol_handler(&self) -> BlobsProtocol {
        BlobsProtocol::new(&self.store, None)
    }

    /// This channel's current manifest root, if we've recorded any messages
    /// for it yet -- `None` means nothing has been recorded (e.g. a channel
    /// just joined with no local history), which is also a valid state to
    /// receive backfill into.
    pub fn current_root(&self, channel: &str) -> Option<Hash> {
        self.channels
            .lock()
            .expect("channels lock poisoned")
            .get(channel)
            .map(|manifest| manifest.root)
    }

    /// Adds `message` to `channel`'s manifest -- a convenience wrapper
    /// around `record_messages` for a single message; see there for details.
    pub async fn record_message(
        &self,
        channel: &str,
        message: &ChatMessage,
    ) -> anyhow::Result<Hash> {
        self.record_messages(channel, std::slice::from_ref(message))
            .await
    }

    /// Adds `messages` to `channel`'s manifest: each becomes its own blob
    /// (idempotent -- content-addressed stores dedupe this for free), is
    /// inserted into the channel's canonical list ordered by
    /// `(ts_unix_ms, id)` (not insertion order -- see `ChannelManifest`),
    /// and the manifest `HashSeq` is rebuilt once from that updated list.
    ///
    /// Called both to seed a channel from storage.rs's already-loaded
    /// history at startup, and after every live message sent or received
    /// during a session, and after every backfill merge. All three call
    /// sites matter: without updating on live messages too, a peer's
    /// announced root would only ever reflect what was true at startup, so
    /// a `HistoryAnnounce` comparison could never detect messages sent
    /// since then -- exactly the gap that let two freshly-started peers
    /// with identical (empty) startup roots never notice one of them had
    /// since sent real messages.
    pub async fn record_messages(
        &self,
        channel: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<Hash> {
        let mut new_entries = Vec::with_capacity(messages.len());
        for message in messages {
            let hash = self.add_message_blob(message).await?;
            new_entries.push((message.ts_unix_ms, message.id, hash));
        }

        let mut entries = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .get(channel)
            .map(|manifest| manifest.entries.clone())
            .unwrap_or_default();
        entries.extend(new_entries);
        entries.sort_by_key(|&(ts, id, _)| (ts, id));
        entries.dedup_by_key(|&mut (ts, id, _)| (ts, id));

        let manifest: HashSeq = entries.iter().map(|&(_, _, hash)| hash).collect();
        let tag = self
            .store
            .blobs()
            .add_bytes_with_opts((manifest.into_inner(), BlobFormat::HashSeq))
            .await
            .map_err(|err| anyhow::anyhow!("failed to add history manifest blob: {err}"))?;

        self.channels
            .lock()
            .expect("channels lock poisoned")
            .insert(
                channel.to_string(),
                ChannelManifest {
                    entries,
                    root: tag.hash,
                },
            );
        Ok(tag.hash)
    }

    async fn add_message_blob(&self, message: &ChatMessage) -> anyhow::Result<Hash> {
        let bytes = postcard::to_stdvec(message).context("failed to encode chat message")?;
        let tag = self
            .store
            .blobs()
            .add_bytes(bytes)
            .await
            .map_err(|err| anyhow::anyhow!("failed to add message blob: {err}"))?;
        Ok(tag.hash)
    }

    /// Fetches `root`'s manifest and every message blob it references from
    /// `sender` -- the peer that announced it, not gossip's `delivered_from`,
    /// which is only a relaying neighbor and may not hold the data itself
    /// (see `crate::message::HistoryAnnounce`).
    ///
    /// Requesting `root` tagged as a `HashSeq` pulls the manifest *and*
    /// every child blob we don't already have in a single call -- content
    /// already present locally (from a previous session or an earlier
    /// merge) is skipped automatically, which is what gives this "pull the
    /// delta" behavior without any manual diffing on our part.
    ///
    /// Malformed entries are logged and skipped rather than failing the
    /// whole backfill, matching storage.rs's tolerance for a best-effort,
    /// not-a-source-of-truth log.
    pub async fn fetch(
        &self,
        endpoint: &Endpoint,
        channel: &str,
        root: Hash,
        sender: [u8; 32],
    ) -> anyhow::Result<Vec<ChatMessage>> {
        let sender_id = EndpointId::from_bytes(&sender)
            .map_err(|err| anyhow::anyhow!("invalid sender id in history announce: {err}"))?;

        self.store
            .downloader(endpoint)
            .download(HashAndFormat::hash_seq(root), vec![sender_id])
            .await
            .map_err(|err| anyhow::anyhow!("failed to download history for #{channel}: {err}"))?;

        let manifest_bytes =
            self.store.blobs().get_bytes(root).await.map_err(|err| {
                anyhow::anyhow!("failed to read downloaded history manifest: {err}")
            })?;
        let manifest = HashSeq::try_from(manifest_bytes)
            .map_err(|err| anyhow::anyhow!("downloaded history manifest is invalid: {err}"))?;

        let mut messages = Vec::with_capacity(manifest.len());
        for hash in manifest.iter() {
            match self.store.blobs().get_bytes(hash).await {
                Ok(bytes) => match postcard::from_bytes::<ChatMessage>(&bytes) {
                    Ok(message) => messages.push(message),
                    Err(err) => warn!(%channel, "dropping malformed backfilled message: {err}"),
                },
                Err(err) => warn!(%channel, "missing backfilled message blob: {err}"),
            }
        }
        Ok(messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(ts_unix_ms: u64, id: u64, text: &str) -> ChatMessage {
        ChatMessage {
            v: 1,
            id,
            sender: [1; 32],
            ts_unix_ms,
            text: text.to_string(),
        }
    }

    #[tokio::test]
    async fn record_messages_root_is_independent_of_input_order() {
        let dir = tempfile::tempdir().unwrap();
        let backfill = BackfillStore::new(dir.path()).await.unwrap();

        let a = message(100, 1, "first");
        let b = message(200, 2, "second");
        let c = message(150, 3, "third");

        let root_forward = backfill
            .record_messages("general", &[a.clone(), b.clone(), c.clone()])
            .await
            .unwrap();
        let root_shuffled = backfill.record_messages("other", &[b, a, c]).await.unwrap();

        assert_eq!(
            root_forward, root_shuffled,
            "two peers with the same message set must compute the same root, \
             regardless of arrival order"
        );
    }

    #[tokio::test]
    async fn record_messages_root_changes_when_the_message_set_changes() {
        let dir = tempfile::tempdir().unwrap();
        let backfill = BackfillStore::new(dir.path()).await.unwrap();

        let root_with_one = backfill
            .record_messages("general", &[message(100, 1, "first")])
            .await
            .unwrap();
        let root_with_two = backfill
            .record_messages("general", &[message(200, 2, "second")])
            .await
            .unwrap();

        assert_ne!(root_with_one, root_with_two);
    }

    #[tokio::test]
    async fn current_root_reflects_the_most_recent_record_and_is_per_channel() {
        let dir = tempfile::tempdir().unwrap();
        let backfill = BackfillStore::new(dir.path()).await.unwrap();

        assert_eq!(backfill.current_root("general"), None);

        let root = backfill
            .record_messages("general", &[message(100, 1, "first")])
            .await
            .unwrap();
        assert_eq!(backfill.current_root("general"), Some(root));
        assert_eq!(
            backfill.current_root("random"),
            None,
            "recording into one channel must not affect another"
        );
    }

    /// Regression test for the bug where a peer's announced root only ever
    /// reflected what was true at startup: recording messages one at a time
    /// as they're sent live must land on the same root as a peer who
    /// received the same messages as one batch (e.g. via backfill or at
    /// startup from a persisted log). Without incremental updates on every
    /// live send/receive, two freshly-started peers would each be stuck
    /// announcing an empty root forever, and a `HistoryAnnounce` comparison
    /// would never notice one side had since sent real messages.
    #[tokio::test]
    async fn recording_messages_one_at_a_time_matches_recording_them_as_one_batch() {
        let dir = tempfile::tempdir().unwrap();
        let incremental = BackfillStore::new(dir.path().join("incremental"))
            .await
            .unwrap();
        let batched = BackfillStore::new(dir.path().join("batched"))
            .await
            .unwrap();

        let messages = [
            message(100, 1, "first"),
            message(200, 2, "second"),
            message(150, 3, "third"),
        ];

        let mut last_root = None;
        for m in &messages {
            last_root = Some(incremental.record_message("general", m).await.unwrap());
        }
        let batch_root = batched.record_messages("general", &messages).await.unwrap();

        assert_eq!(
            last_root.unwrap(),
            batch_root,
            "a root built up message-by-message (live sends) must match a root \
             built from the same messages all at once (a fresh peer's startup \
             seed or a backfill merge)"
        );
    }
}
