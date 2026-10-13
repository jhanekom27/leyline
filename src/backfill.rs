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
use std::time::Duration;

use anyhow::Context;
use iroh::{Endpoint, EndpointId};
use iroh_blobs::api::proto::BlobStatus;
use iroh_blobs::get::request::get_verified_size;
use iroh_blobs::hashseq::HashSeq;
use iroh_blobs::store::fs::FsStore;
use iroh_blobs::{BlobFormat, BlobsProtocol, Hash, HashAndFormat};
use tracing::{debug, warn};

use crate::files::human_size;
use crate::message::ChatMessage;

/// What a save says when no provider could be reached or had the file.
const NOBODY_HAS_IT: &str = "no reachable peer has it right now -- its sender may be offline";

/// How long one provider gets to be reached and to say how big a file is
/// (`verified_size`) before the next is tried. Generous next to the
/// downloader's one-second connect timeout: this runs for a save nobody is
/// waiting on, and a probe that gave up early would fail an auto-save that
/// `/save` would have managed.
const SIZE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

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

    /// Forgets `channel`'s in-memory manifest tracking -- e.g. when the
    /// user runs `/leave` (see `app::AppState::run_leave`) -- so
    /// `current_root` reverts to `None` and a later rejoin under the same
    /// display name starts from an empty root instead of announcing an
    /// unrelated room's history as this one's (see `storage.rs`'s
    /// `delete` for the same concern on the local message log). Doesn't
    /// reclaim the underlying message/manifest blobs already written to
    /// the blob store -- they're content-addressed and harmless to keep,
    /// and garbage-collecting them isn't worth the complexity here.
    pub fn forget_channel(&self, channel: &str) {
        self.channels
            .lock()
            .expect("channels lock poisoned")
            .remove(channel);
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
                // `ChatMessage::decode` (not a plain `postcard::from_bytes`)
                // so a peer still on an older build -- whose blobs predate
                // `attachment` -- doesn't get silently dropped here.
                Ok(bytes) => match ChatMessage::decode(&bytes) {
                    Ok(message) => messages.push(message),
                    Err(err) => warn!(%channel, "dropping malformed backfilled message: {err}"),
                },
                Err(err) => warn!(%channel, "missing backfilled message blob: {err}"),
            }
        }
        Ok(messages)
    }

    /// Imports a local file into the shared blob store for `/send` -- see
    /// features.md's "File sharing" idea. Streamed from disk (no need to
    /// buffer the whole file in memory); the original file at `path` is
    /// left untouched, though its bytes are copied into this store's own
    /// directory, the same tradeoff already accepted above for
    /// message/manifest blobs.
    pub async fn add_file(&self, path: &Path) -> anyhow::Result<Hash> {
        let tag = self
            .store
            .blobs()
            .add_path(path)
            .await
            .map_err(|err| anyhow::anyhow!("failed to import {}: {err}", path.display()))?;
        Ok(tag.hash)
    }

    /// Imports in-memory bytes into the shared blob store -- the `/paste`
    /// counterpart to `add_file`, for content that only ever exists as
    /// bytes (a clipboard image re-encoded to PNG, see `net::encode_png`),
    /// never a real path on disk. A thin public twin of
    /// `add_message_blob` above, which already wraps the same
    /// `store.blobs().add_bytes` call for message/manifest blobs.
    pub async fn add_bytes(&self, bytes: Vec<u8>) -> anyhow::Result<Hash> {
        let tag = self
            .store
            .blobs()
            .add_bytes(bytes)
            .await
            .map_err(|err| anyhow::anyhow!("failed to import clipboard content: {err}"))?;
        Ok(tag.hash)
    }

    /// Fetches a single file blob and writes it straight to `destination`
    /// -- the `/save` counterpart to `add_file`, using the same
    /// pull-on-demand downloader as `fetch` above, just for one raw blob
    /// instead of a history manifest.
    ///
    /// `providers` are asked one at a time, in the order given, until one
    /// serves the whole blob -- the file's sender first, then the other
    /// peers known in its channel (see `app::AppState::providers_for`).
    /// Anyone who already saved the file holds it in their own blob store
    /// and serves it from there: a download lands in the store first and
    /// `destination` only gets an exported copy, so the file outlives its
    /// sender's session, and moving or deleting that copy changes nothing
    /// for whoever else wants it. A provider that is offline, or online
    /// without the blob, only hands over to the next one. A blob already
    /// in our own store -- a file we sent, or saved before -- is exported
    /// without asking anyone.
    ///
    /// The ids come from peers' messages, so one that is not a valid key is
    /// skipped rather than failing the save for everyone else listed.
    ///
    /// `max_bytes`, if given, is the most the file may be -- of what it *is*,
    /// never of what its sender announced, which is only a number in a
    /// message. A file already in our own store is measured there; one that
    /// is not is sized first by asking the providers (`verified_size`), and
    /// nothing is downloaded -- nor, for the first, exported -- if it is over.
    pub async fn fetch_file(
        &self,
        endpoint: &Endpoint,
        hash: Hash,
        providers: &[[u8; 32]],
        destination: &Path,
        max_bytes: Option<u64>,
    ) -> anyhow::Result<()> {
        let local = self
            .store
            .blobs()
            .status(hash)
            .await
            .map_err(|err| anyhow::anyhow!("failed to look for the file locally: {err}"))?;

        match local {
            BlobStatus::Complete { size } => {
                if let Some(limit) = max_bytes {
                    ensure_within(size, limit)?;
                }
            }
            BlobStatus::NotFound | BlobStatus::Partial { .. } => {
                let mut providers: Vec<EndpointId> = providers
                    .iter()
                    .filter_map(|id| EndpointId::from_bytes(id).ok())
                    .collect();

                if let Some(limit) = max_bytes {
                    let (answered, size) = verified_size(endpoint, hash, &providers)
                        .await
                        .ok_or_else(|| anyhow::anyhow!(NOBODY_HAS_IT))?;
                    ensure_within(size, limit)?;
                    // The one that just answered has the file: ask it first.
                    providers.retain(|id| *id != answered);
                    providers.insert(0, answered);
                }

                self.store
                    .downloader(endpoint)
                    .download(HashAndFormat::raw(hash), providers)
                    .await
                    .map_err(|err| {
                        // The downloader only says it was "unable to download"
                        // the hash once every provider has failed; what each
                        // one answered stays in its own trace. Nobody can act
                        // on that, so the user is told what is going on.
                        debug!(%hash, "no provider served the file: {err}");
                        anyhow::anyhow!(NOBODY_HAS_IT)
                    })?;
            }
        }

        self.store
            .blobs()
            .export(hash, destination)
            .await
            .map_err(|err| anyhow::anyhow!("failed to write {}: {err}", destination.display()))?;
        Ok(())
    }
}

/// Refuses a file of `size` bytes if it is over `limit`, saying how big it is.
fn ensure_within(size: u64, limit: u64) -> anyhow::Result<()> {
    anyhow::ensure!(
        size <= limit,
        "it is {}, over the {} limit",
        human_size(size),
        human_size(limit)
    );
    Ok(())
}

/// How big the blob `hash` is, as the first of `providers` that can be
/// reached and has it says -- and which one that was. The protocol checks the
/// answer against the hash itself (only the blob's last chunk and its proof
/// are read, none of its content), so unlike the size a peer wrote in a
/// message it can't be a lie. `None` if no provider answers.
async fn verified_size(
    endpoint: &Endpoint,
    hash: Hash,
    providers: &[EndpointId],
) -> Option<(EndpointId, u64)> {
    for &provider in providers {
        let probe = async {
            let connection = endpoint
                .connect(provider, iroh_blobs::ALPN)
                .await
                .map_err(|err| err.to_string())?;
            get_verified_size(&connection, &hash)
                .await
                .map(|(size, _stats)| size)
                .map_err(|err| err.to_string())
        };
        match tokio::time::timeout(SIZE_PROBE_TIMEOUT, probe).await {
            Ok(Ok(size)) => return Some((provider, size)),
            Ok(Err(err)) => debug!(%hash, %provider, "provider could not size the file: {err}"),
            Err(_) => debug!(%hash, %provider, "provider took too long to size the file"),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use iroh::address_lookup::memory::MemoryLookup;
    use iroh::endpoint::presets;
    use iroh::protocol::Router;

    use super::*;

    fn message(ts_unix_ms: u64, id: u64, text: &str) -> ChatMessage {
        ChatMessage {
            v: 1,
            id,
            sender: [1; 32],
            ts_unix_ms,
            text: text.to_string(),
            attachment: None,
            reply_to: None,
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

    #[tokio::test]
    async fn forget_channel_clears_the_current_root() {
        let dir = tempfile::tempdir().unwrap();
        let backfill = BackfillStore::new(dir.path()).await.unwrap();
        backfill
            .record_messages("general", &[message(100, 1, "first")])
            .await
            .unwrap();

        backfill.forget_channel("general");

        assert_eq!(backfill.current_root("general"), None);
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

    #[tokio::test]
    async fn add_bytes_imports_in_memory_content_and_returns_its_hash() {
        let dir = tempfile::tempdir().unwrap();
        let backfill = BackfillStore::new(dir.path()).await.unwrap();

        let hash = backfill
            .add_bytes(b"fake png bytes".to_vec())
            .await
            .unwrap();

        let bytes = backfill.store.blobs().get_bytes(hash).await.unwrap();
        assert_eq!(bytes.as_ref(), b"fake png bytes");
    }

    #[tokio::test]
    async fn add_bytes_is_content_addressed_like_add_file() {
        let dir = tempfile::tempdir().unwrap();
        let backfill = BackfillStore::new(dir.path()).await.unwrap();

        let first = backfill.add_bytes(b"same content".to_vec()).await.unwrap();
        let second = backfill.add_bytes(b"same content".to_vec()).await.unwrap();

        assert_eq!(first, second, "identical bytes must hash identically");
    }

    /// A running peer for the file-fetch tests: its own blob store served
    /// over a real endpoint -- the same pieces net.rs wires together --
    /// bound with no relay or discovery, so each test decides who can find
    /// whom (`learn`) without depending on public infrastructure.
    struct TestPeer {
        backfill: BackfillStore,
        router: Router,
        lookup: MemoryLookup,
        /// Keeps the store's directory alive as long as the peer.
        _dir: tempfile::TempDir,
    }

    impl TestPeer {
        async fn start() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let backfill = BackfillStore::new(dir.path()).await.unwrap();
            let lookup = MemoryLookup::new();
            let endpoint = Endpoint::builder(presets::Minimal)
                .address_lookup(lookup.clone())
                .bind()
                .await
                .unwrap();
            let router = Router::builder(endpoint)
                .accept(iroh_blobs::ALPN, backfill.protocol_handler())
                .spawn();
            Self {
                backfill,
                router,
                lookup,
                _dir: dir,
            }
        }

        fn id(&self) -> [u8; 32] {
            *self.router.endpoint().id().as_bytes()
        }

        /// Lets this peer dial `other` by its id alone, as net.rs can for
        /// anyone it holds an address hint for. Done while `other` is up: a
        /// peer that has gone offline has no address left to give.
        fn learn(&self, other: &TestPeer) {
            self.lookup
                .add_endpoint_info(other.router.endpoint().addr());
        }

        /// Puts `bytes` into this peer's blob store, as sending a file does.
        async fn share(&self, bytes: &[u8]) -> Hash {
            self.backfill.add_bytes(bytes.to_vec()).await.unwrap()
        }

        /// What `/save` does: fetches `hash` from `providers`, in order,
        /// and exports it to `destination`.
        async fn save(
            &self,
            hash: Hash,
            providers: &[[u8; 32]],
            destination: &Path,
        ) -> anyhow::Result<()> {
            self.backfill
                .fetch_file(self.router.endpoint(), hash, providers, destination, None)
                .await
        }

        /// What an auto-save does: `save`, but refusing a file of more than
        /// `limit` bytes.
        async fn save_within(
            &self,
            hash: Hash,
            providers: &[[u8; 32]],
            destination: &Path,
            limit: u64,
        ) -> anyhow::Result<()> {
            self.backfill
                .fetch_file(
                    self.router.endpoint(),
                    hash,
                    providers,
                    destination,
                    Some(limit),
                )
                .await
        }

        /// Whether `hash` is anywhere in this peer's blob store, even in part.
        async fn holds_any_of(&self, hash: Hash) -> bool {
            self.backfill.store.blobs().status(hash).await.unwrap() != BlobStatus::NotFound
        }

        /// The peer going away: its endpoint closes and nothing answers on
        /// its id any more.
        async fn go_offline(&self) {
            self.router.shutdown().await.unwrap();
        }
    }

    /// Regression test for the multi-device identity story (LEY-20):
    /// `/save` fetches a file by dialing endpoint ids directly (see
    /// `net::Net::save_file`), never "the channel" at large, so sending
    /// from one device and saving from another is really just this -- a
    /// different device's `BackfillStore` fetching from the sender's real,
    /// running endpoint over loopback, not merely one store's own round
    /// trip (the other tests in this file never involve a second store or
    /// any real networking).
    #[tokio::test]
    async fn fetch_file_retrieves_a_file_from_a_different_devices_backfill_store() {
        let sender = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&sender);

        let file_dir = tempfile::tempdir().unwrap();
        let file_path = file_dir.path().join("report.txt");
        std::fs::write(&file_path, b"shared across devices").unwrap();
        let hash = sender.backfill.add_file(&file_path).await.unwrap();

        let destination = file_dir.path().join("downloaded.txt");
        receiver
            .save(hash, &[sender.id()], &destination)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"shared across devices"
        );
    }

    /// LEY-25.5: a file outlives the session of whoever sent it. Anyone who
    /// saved it serves it from their own blob store, so it can still be
    /// fetched from them once the sender is gone -- and once they have
    /// moved or deleted the copy in their Downloads folder, which is only an
    /// export out of the store, never what gets served.
    #[tokio::test]
    async fn fetch_file_falls_back_to_a_peer_that_saved_it_when_the_sender_is_offline() {
        let sender = TestPeer::start().await;
        let saver = TestPeer::start().await;
        let latecomer = TestPeer::start().await;
        saver.learn(&sender);
        latecomer.learn(&sender);
        latecomer.learn(&saver);

        // Bigger than a store keeps inline in its database, so this is a
        // file on disk inside the store, like any real share -- the case a
        // moved Downloads copy would matter for.
        let content = vec![7u8; 64 * 1024];
        let hash = sender.share(&content).await;

        let downloads = tempfile::tempdir().unwrap();
        let saved = downloads.path().join("report.bin");
        saver.save(hash, &[sender.id()], &saved).await.unwrap();
        std::fs::remove_file(&saved).unwrap();
        sender.go_offline().await;

        let destination = downloads.path().join("late.bin");
        latecomer
            .save(hash, &[sender.id(), saver.id()], &destination)
            .await
            .unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), content);
    }

    #[tokio::test]
    async fn fetch_file_skips_an_online_peer_that_does_not_have_the_file() {
        // Online and reachable, but it never saw the file: listed ahead of
        // the one that has it, it must not fail the save.
        let bystander = TestPeer::start().await;
        let holder = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&bystander);
        receiver.learn(&holder);
        let hash = holder.share(b"only the holder has this").await;

        let downloads = tempfile::tempdir().unwrap();
        let destination = downloads.path().join("report.txt");
        receiver
            .save(hash, &[bystander.id(), holder.id()], &destination)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"only the holder has this"
        );
    }

    #[tokio::test]
    async fn fetch_file_says_no_reachable_peer_has_it_when_every_provider_fails() {
        let sender = TestPeer::start().await;
        let bystander = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&sender);
        receiver.learn(&bystander);
        let hash = sender.share(b"nobody left to serve this").await;
        sender.go_offline().await;

        let downloads = tempfile::tempdir().unwrap();
        let destination = downloads.path().join("report.txt");
        let err = receiver
            .save(hash, &[sender.id(), bystander.id()], &destination)
            .await
            .unwrap_err();

        let message = err.to_string();
        assert!(
            message.contains("no reachable peer has it"),
            "got: {message}"
        );
        assert!(message.contains("sender may be offline"), "got: {message}");
        assert!(
            !message.contains("Unable to download"),
            "the downloader's own wording must not reach the user, got: {message}"
        );
        assert!(
            !destination.exists(),
            "a failed save must not leave a file behind"
        );
    }

    #[tokio::test]
    async fn fetch_file_saves_a_file_already_in_the_local_store_whatever_the_providers() {
        // A file we sent ourselves, or saved before, is in our own store:
        // saving it again needs no peer, so none being reachable -- or
        // listed at all -- is fine.
        let gone = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&gone);
        gone.go_offline().await;
        let hash = receiver.share(b"already here").await;

        let downloads = tempfile::tempdir().unwrap();
        let unreachable = downloads.path().join("unreachable.txt");
        receiver
            .save(hash, &[gone.id()], &unreachable)
            .await
            .unwrap();
        let unlisted = downloads.path().join("unlisted.txt");
        receiver.save(hash, &[], &unlisted).await.unwrap();

        assert_eq!(std::fs::read(&unreachable).unwrap(), b"already here");
        assert_eq!(std::fs::read(&unlisted).unwrap(), b"already here");
    }

    #[tokio::test]
    async fn fetch_file_skips_a_provider_id_that_is_not_a_valid_key() {
        // Provider ids come from peers' messages, so a crafted one must not
        // be able to break `/save` for everyone else in the channel.
        let invalid = (0..=u8::MAX)
            .map(|byte| [byte; 32])
            .find(|id| EndpointId::from_bytes(id).is_err())
            .expect("some repeated-byte id is not a valid key");
        let holder = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&holder);
        let hash = holder.share(b"served despite the junk id").await;

        let downloads = tempfile::tempdir().unwrap();
        let destination = downloads.path().join("report.txt");
        receiver
            .save(hash, &[invalid, holder.id()], &destination)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"served despite the junk id"
        );

        // With nobody valid to ask it fails like any other save that has no
        // one to serve it.
        let err = receiver
            .save(
                Hash::new(b"held by no one"),
                &[invalid],
                &downloads.path().join("other.txt"),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("no reachable peer has it"),
            "got: {err}"
        );
    }

    /// Bigger than a store keeps inline in its database, like any real
    /// share: 64 KiB.
    fn big_file() -> Vec<u8> {
        vec![7u8; 64 * 1024]
    }

    #[tokio::test]
    async fn a_file_over_the_limit_is_refused_and_nothing_is_downloaded() {
        let sender = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&sender);
        let hash = sender.share(&big_file()).await;

        let downloads = tempfile::tempdir().unwrap();
        let destination = downloads.path().join("big.bin");
        let err = receiver
            .save_within(hash, &[sender.id()], &destination, 1024)
            .await
            .unwrap_err();

        let message = err.to_string();
        assert!(
            message.contains("64.0 KB") && message.contains("over the 1.0 KB limit"),
            "it says how big the file is and what the limit was, got: {message}"
        );
        assert!(!destination.exists(), "nothing is written");
        assert!(
            !receiver.holds_any_of(hash).await,
            "not a byte of it was downloaded into the store either"
        );
    }

    #[tokio::test]
    async fn the_limit_is_exact() {
        let sender = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&sender);
        let content = big_file();
        let hash = sender.share(&content).await;
        let size = content.len() as u64;

        let downloads = tempfile::tempdir().unwrap();
        let one_short = downloads.path().join("refused.bin");
        receiver
            .save_within(hash, &[sender.id()], &one_short, size - 1)
            .await
            .unwrap_err();
        let exactly = downloads.path().join("exactly.bin");
        receiver
            .save_within(hash, &[sender.id()], &exactly, size)
            .await
            .unwrap();

        assert!(!one_short.exists());
        assert_eq!(std::fs::read(&exactly).unwrap(), content);
    }

    #[tokio::test]
    async fn a_file_within_the_limit_is_saved_like_any_other() {
        let sender = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&sender);
        let hash = sender.share(b"small enough").await;

        let downloads = tempfile::tempdir().unwrap();
        let destination = downloads.path().join("small.txt");
        receiver
            .save_within(hash, &[sender.id()], &destination, 1024)
            .await
            .unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"small enough");
    }

    #[tokio::test]
    async fn the_limit_also_bounds_a_file_already_in_the_local_store() {
        // Exporting a copy of what is already here still puts that many
        // bytes on disk, so it is held to the limit like anything else.
        let receiver = TestPeer::start().await;
        let content = big_file();
        let hash = receiver.share(&content).await;

        let downloads = tempfile::tempdir().unwrap();
        let refused = downloads.path().join("refused.bin");
        let err = receiver
            .save_within(hash, &[], &refused, 1024)
            .await
            .unwrap_err();
        let allowed = downloads.path().join("allowed.bin");
        receiver
            .save_within(hash, &[], &allowed, 1 << 20)
            .await
            .unwrap();

        assert!(
            err.to_string().contains("over the 1.0 KB limit"),
            "got: {err}"
        );
        assert!(!refused.exists());
        assert_eq!(std::fs::read(&allowed).unwrap(), content);
    }

    #[tokio::test]
    async fn a_provider_without_the_file_does_not_stop_it_being_sized_by_the_next() {
        let bystander = TestPeer::start().await;
        let holder = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&bystander);
        receiver.learn(&holder);
        let content = big_file();
        let hash = holder.share(&content).await;
        let providers = [bystander.id(), holder.id()];

        let downloads = tempfile::tempdir().unwrap();
        let allowed = downloads.path().join("allowed.bin");
        receiver
            .save_within(hash, &providers, &allowed, 1 << 20)
            .await
            .unwrap();
        let too_big = holder.share(b"x".repeat(4096).as_slice()).await;
        let refused = downloads.path().join("refused.bin");
        let err = receiver
            .save_within(too_big, &providers, &refused, 1024)
            .await
            .unwrap_err();

        assert_eq!(std::fs::read(&allowed).unwrap(), content);
        assert!(
            err.to_string().contains("over the 1.0 KB limit"),
            "got: {err}"
        );
        assert!(!refused.exists());
        assert!(
            !receiver.holds_any_of(too_big).await,
            "nothing over the limit reached the store"
        );
    }

    #[tokio::test]
    async fn with_a_limit_a_file_nobody_has_still_says_no_reachable_peer_has_it() {
        let bystander = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&bystander);

        let downloads = tempfile::tempdir().unwrap();
        let destination = downloads.path().join("report.txt");
        let err = receiver
            .save_within(
                Hash::new(b"held by no one"),
                &[bystander.id()],
                &destination,
                1 << 20,
            )
            .await
            .unwrap_err();

        let message = err.to_string();
        assert!(
            message.contains("no reachable peer has it")
                && message.contains("sender may be offline"),
            "got: {message}"
        );
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn with_a_limit_a_provider_id_that_is_not_a_valid_key_is_skipped() {
        let invalid = (0..=u8::MAX)
            .map(|byte| [byte; 32])
            .find(|id| EndpointId::from_bytes(id).is_err())
            .expect("some repeated-byte id is not a valid key");
        let holder = TestPeer::start().await;
        let receiver = TestPeer::start().await;
        receiver.learn(&holder);
        let hash = holder.share(b"served despite the junk id").await;

        let downloads = tempfile::tempdir().unwrap();
        let destination = downloads.path().join("report.txt");
        receiver
            .save_within(hash, &[invalid, holder.id()], &destination, 1024)
            .await
            .unwrap();

        assert_eq!(
            std::fs::read(&destination).unwrap(),
            b"served despite the junk id"
        );
    }
}
