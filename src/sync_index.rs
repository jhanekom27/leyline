//! Per-channel replicated file-index documents.
//!
//! This module owns the iroh-docs service and its live subscriptions.  It
//! deliberately exposes index changes as runtime events only: applying an
//! entry to a channel folder belongs to a later filesystem-focused feature.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use futures::StreamExt;
use iroh::EndpointAddr;
use iroh_blobs::{Hash, store::fs::FsStore};
use iroh_docs::{
    AuthorId, Capability, ContentStatus,
    api::Doc,
    api::protocol::{AddrInfoOptions, ShareMode},
    protocol::Docs,
    store::Query,
};
use iroh_gossip::net::Gossip;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;

use crate::net::{FileIndexEntry, NetEvent, RemoteIndexEntry};

struct ActiveIndex {
    doc: Doc,
    capability: Capability,
    watch_task: JoinHandle<()>,
}

/// The network-owned iroh-docs service shared by all channel indexes.
#[derive(Clone)]
pub struct SyncIndex {
    docs: Docs,
    author: AuthorId,
    indexes: Arc<Mutex<HashMap<String, ActiveIndex>>>,
    events_tx: mpsc::Sender<NetEvent>,
    blobs: FsStore,
}

impl SyncIndex {
    /// Spawns one persistent docs service on the existing endpoint/blob/gossip
    /// substrate.  Its protocol is registered on the same router as the
    /// existing network services.
    pub async fn start(
        dir: impl AsRef<Path>,
        endpoint: iroh::Endpoint,
        blobs: FsStore,
        gossip: Gossip,
        events_tx: mpsc::Sender<NetEvent>,
    ) -> anyhow::Result<(Self, Docs)> {
        let dir = dir.as_ref();
        std::fs::create_dir_all(dir)
            .with_context(|| format!("failed to create docs store at {}", dir.display()))?;
        let docs = Docs::persistent(dir.to_path_buf())
            .spawn(endpoint, blobs.clone().into(), gossip)
            .await
            .context("failed to start iroh-docs")?;
        let author = docs
            .author_default()
            .await
            .context("failed to load docs author")?;
        let protocol = docs.clone();
        Ok((
            Self {
                docs,
                author,
                indexes: Arc::new(Mutex::new(HashMap::new())),
                events_tx,
                blobs,
            },
            protocol,
        ))
    }

    /// Opens (or creates) a channel's writable document and starts
    /// reconciliation with `peers`.  Returns the capability that must be
    /// persisted with the channel and propagated in invitations/pairing.
    pub async fn join(
        &self,
        channel: &str,
        capability: Option<Capability>,
        peers: Vec<EndpointAddr>,
    ) -> anyhow::Result<Capability> {
        if let Some(capability) = self.capability_for(channel) {
            return Ok(capability);
        }

        let (doc, capability) = match capability {
            Some(capability) => {
                let doc = self
                    .docs
                    .import_namespace(capability.clone())
                    .await
                    .context("failed to import channel index capability")?;
                (doc, capability)
            }
            None => {
                let doc = self
                    .docs
                    .create()
                    .await
                    .context("failed to create channel index")?;
                let ticket = doc
                    .share(ShareMode::Write, AddrInfoOptions::Id)
                    .await
                    .context("failed to create channel index capability")?;
                (doc, ticket.capability)
            }
        };
        let stream = doc
            .subscribe()
            .await
            .context("failed to subscribe to channel index")?;
        doc.start_sync(peers)
            .await
            .context("failed to start channel index sync")?;
        let watch_task = watch(
            channel.to_string(),
            stream,
            self.events_tx.clone(),
            self.blobs.clone(),
        );
        self.indexes.lock().expect("indexes lock poisoned").insert(
            channel.to_string(),
            ActiveIndex {
                doc,
                capability: capability.clone(),
                watch_task,
            },
        );
        Ok(capability)
    }

    pub fn capability_for(&self, channel: &str) -> Option<Capability> {
        self.indexes
            .lock()
            .expect("indexes lock poisoned")
            .get(channel)
            .map(|index| index.capability.clone())
    }

    /// Rebinds `channel` to a capability learned from an existing member.
    /// Legacy peers can have independently-created local documents for the
    /// same gossip channel; abandoning the noncanonical namespace lets the
    /// channel converge on one index without changing its chat membership.
    pub async fn adopt(
        &self,
        channel: &str,
        capability: Capability,
        peers: Vec<EndpointAddr>,
    ) -> anyhow::Result<Capability> {
        let existing = self
            .indexes
            .lock()
            .expect("indexes lock poisoned")
            .remove(channel);
        if let Some(index) = existing {
            if index.capability.raw() == capability.raw() {
                index
                    .doc
                    .start_sync(peers)
                    .await
                    .context("failed to restart channel index sync")?;
                let capability = index.capability.clone();
                self.indexes
                    .lock()
                    .expect("indexes lock poisoned")
                    .insert(channel.to_string(), index);
                return Ok(capability);
            }
            index.watch_task.abort();
            index
                .doc
                .leave()
                .await
                .context("failed to leave divergent channel index")?;
            index
                .doc
                .close()
                .await
                .context("failed to close divergent channel index")?;
            self.docs
                .drop_doc(index.capability.id())
                .await
                .context("failed to remove divergent channel index")?;
        }
        self.join(channel, Some(capability), peers).await
    }

    /// Adds an index entry.  This is intentionally network-only; callers
    /// must not use it to alter a channel folder.
    pub fn set(&self, channel: &str, path: String, metadata: Vec<u8>) -> anyhow::Result<()> {
        let doc = self
            .indexes
            .lock()
            .expect("indexes lock poisoned")
            .get(channel)
            .map(|index| index.doc.clone())
            .ok_or_else(|| anyhow::anyhow!("channel index is not joined: {channel}"))?;
        let author = self.author;
        tokio::spawn(async move {
            if let Err(err) = doc.set_bytes(author, path, metadata).await {
                warn!("failed to update channel index: {err}");
            }
        });
        Ok(())
    }

    /// Publishes a tombstone for one path so offline peers converge later.
    pub fn delete(&self, channel: &str, path: String) -> anyhow::Result<()> {
        let doc = self
            .indexes
            .lock()
            .expect("indexes lock poisoned")
            .get(channel)
            .map(|index| index.doc.clone())
            .ok_or_else(|| anyhow::anyhow!("channel index is not joined: {channel}"))?;
        let author = self.author;
        tokio::spawn(async move {
            if let Err(err) = doc.del(author, path).await {
                warn!("failed to delete channel index entry: {err}");
            }
        });
        Ok(())
    }

    /// Whether the resolved entry for `path` already names `content_hash`.
    /// Local publication uses this to recognize a file that sync just wrote
    /// into the folder and avoid turning that remote update into a new write.
    pub async fn matches_content(
        &self,
        channel: &str,
        path: &str,
        content_hash: iroh_blobs::Hash,
    ) -> anyhow::Result<bool> {
        let doc = self
            .indexes
            .lock()
            .expect("indexes lock poisoned")
            .get(channel)
            .map(|index| index.doc.clone())
            .ok_or_else(|| anyhow::anyhow!("channel index is not joined: {channel}"))?;
        let Some(entry) = doc
            .get_one(Query::single_latest_per_key().key_exact(path.as_bytes()))
            .await
            .context("failed to resolve channel index entry")?
        else {
            return Ok(false);
        };
        let metadata = self
            .blobs
            .blobs()
            .get_bytes(entry.content_hash())
            .await
            .context("failed to read resolved channel index metadata")?;
        let entry: FileIndexEntry = postcard::from_bytes(&metadata)
            .context("failed to decode resolved channel index metadata")?;
        Ok(entry.content_hash == content_hash)
    }

    /// Stops live reconciliation and permanently removes local index state.
    /// The remote namespace remains available to other channel members.
    pub fn leave(&self, channel: &str) {
        let Some(index) = self
            .indexes
            .lock()
            .expect("indexes lock poisoned")
            .remove(channel)
        else {
            return;
        };
        index.watch_task.abort();
        let docs = self.docs.clone();
        tokio::spawn(async move {
            if let Err(err) = index.doc.leave().await {
                warn!("failed to leave channel index sync: {err}");
            }
            if let Err(err) = index.doc.close().await {
                warn!("failed to close channel index: {err}");
            }
            if let Err(err) = docs.drop_doc(index.capability.id()).await {
                warn!("failed to remove channel index: {err}");
            }
        });
    }
}

fn watch(
    channel: String,
    mut stream: impl futures::Stream<Item = anyhow::Result<iroh_docs::engine::LiveEvent>>
    + Send
    + Unpin
    + 'static,
    events_tx: mpsc::Sender<NetEvent>,
    blobs: FsStore,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut pending = HashMap::<Hash, Vec<String>>::new();
        while let Some(event) = stream.next().await {
            match event {
                Ok(iroh_docs::engine::LiveEvent::InsertRemote {
                    entry,
                    content_status,
                    ..
                }) => {
                    let Ok(path) = String::from_utf8(entry.key().to_vec()) else {
                        warn!(%channel, "dropping non-UTF-8 remote index path");
                        continue;
                    };
                    if entry.content_hash() == Hash::EMPTY {
                        if events_tx
                            .send(NetEvent::RemoteIndexDelete {
                                channel: channel.clone(),
                                path,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                        continue;
                    }
                    let hash = entry.content_hash();
                    if content_status == ContentStatus::Complete {
                        if !send_remote_entry(&channel, path, hash, &blobs, &events_tx).await {
                            break;
                        }
                    } else {
                        pending.entry(hash).or_default().push(path);
                    }
                }
                Ok(iroh_docs::engine::LiveEvent::ContentReady { hash }) => {
                    for path in pending.remove(&hash).unwrap_or_default() {
                        if !send_remote_entry(&channel, path, hash, &blobs, &events_tx).await {
                            return;
                        }
                    }
                }
                Ok(_) => {}
                Err(err) => {
                    warn!(%channel, "channel index event stream failed: {err}");
                    break;
                }
            }
        }
    })
}

async fn send_remote_entry(
    channel: &str,
    path: String,
    metadata_hash: Hash,
    blobs: &FsStore,
    events_tx: &mpsc::Sender<NetEvent>,
) -> bool {
    let metadata = match blobs.blobs().get_bytes(metadata_hash).await {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!(%channel, "failed to read remote index metadata: {err}");
            return true;
        }
    };
    let entry = match postcard::from_bytes::<FileIndexEntry>(&metadata) {
        Ok(entry) => entry,
        Err(err) => {
            warn!(%channel, "dropping malformed remote index metadata: {err}");
            return true;
        }
    };
    events_tx
        .send(NetEvent::RemoteIndex(RemoteIndexEntry {
            channel: channel.to_string(),
            path,
            entry,
        }))
        .await
        .is_ok()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use iroh::{Endpoint, endpoint::presets, protocol::Router};
    use iroh_gossip::net::Gossip;
    use tokio::sync::mpsc;

    use super::*;
    use crate::backfill::BackfillStore;
    use crate::sync_apply;
    use crate::sync_publish::{self, Watcher};

    struct Peer {
        backfill: BackfillStore,
        index: SyncIndex,
        router: Router,
        events: mpsc::Receiver<NetEvent>,
        _dir: tempfile::TempDir,
    }

    impl Peer {
        async fn start() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let backfill = BackfillStore::new(dir.path().join("blobs")).await.unwrap();
            let endpoint = Endpoint::bind(presets::Minimal).await.unwrap();
            let gossip = Gossip::builder().spawn(endpoint.clone());
            let (tx, events) = mpsc::channel(8);
            let (index, docs) = SyncIndex::start(
                dir.path().join("docs"),
                endpoint.clone(),
                backfill.blob_store(),
                gossip.clone(),
                tx,
            )
            .await
            .unwrap();
            let router = Router::builder(endpoint)
                .accept(iroh_blobs::ALPN, backfill.protocol_handler())
                .accept(iroh_gossip::net::GOSSIP_ALPN, gossip)
                .accept(iroh_docs::ALPN, docs)
                .spawn();
            Self {
                backfill,
                index,
                router,
                events,
                _dir: dir,
            }
        }

        async fn shutdown(self) {
            self.router.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn publishes_a_stable_local_folder_file_to_a_remote_peer() {
        let alpha = Peer::start().await;
        let mut beta = Peer::start().await;
        let capability = alpha.index.join("general", None, vec![]).await.unwrap();
        beta.index
            .join(
                "general",
                Some(capability),
                vec![alpha.router.endpoint().addr()],
            )
            .await
            .unwrap();

        let source_root = tempfile::tempdir().unwrap();
        let source = source_root.path().join("general").join("report.txt");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"published from a channel folder").unwrap();
        let mut watcher = Watcher::default();
        assert!(
            watcher
                .stable(
                    sync_publish::scan("general".into(), source_root.path().join("general"))
                        .unwrap()
                )
                .is_empty()
        );
        let change = watcher
            .stable(
                sync_publish::scan("general".into(), source_root.path().join("general")).unwrap(),
            )
            .pop()
            .unwrap();
        let sync_publish::Change::Present(candidate) = change else {
            panic!("stable file must be a present change");
        };
        let hash = alpha.backfill.add_file(&candidate.path).await.unwrap();
        alpha
            .index
            .set(
                "general",
                candidate.relative,
                postcard::to_stdvec(&FileIndexEntry {
                    content_hash: hash,
                    size: candidate.size,
                    modified_unix_ms: 0,
                    author: *alpha.router.endpoint().id().as_bytes(),
                })
                .unwrap(),
            )
            .unwrap();

        let remote = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(NetEvent::RemoteIndex(entry)) = beta.events.recv().await {
                    break entry;
                }
            }
        })
        .await
        .expect("remote index entry did not arrive");
        let destination_root = tempfile::tempdir().unwrap();
        let destination = sync_apply::reserve(destination_root.path(), &remote.path).unwrap();
        beta.backfill
            .fetch_file(
                beta.router.endpoint(),
                remote.entry.content_hash,
                &[remote.entry.author],
                &destination,
                Some(remote.entry.size),
            )
            .await
            .unwrap();

        assert_eq!(
            std::fs::read(destination).unwrap(),
            b"published from a channel folder"
        );
        alpha.shutdown().await;
        beta.shutdown().await;
    }

    #[tokio::test]
    async fn publishes_a_live_tombstone_to_a_remote_peer() {
        let alpha = Peer::start().await;
        let mut beta = Peer::start().await;
        let capability = alpha.index.join("general", None, vec![]).await.unwrap();
        beta.index
            .join(
                "general",
                Some(capability),
                vec![alpha.router.endpoint().addr()],
            )
            .await
            .unwrap();

        alpha.index.delete("general", "removed.txt".into()).unwrap();
        let deleted = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(NetEvent::RemoteIndexDelete { channel, path }) =
                    beta.events.recv().await
                {
                    break (channel, path);
                }
            }
        })
        .await
        .expect("remote tombstone did not arrive");

        assert_eq!(deleted, ("general".into(), "removed.txt".into()));
        alpha.shutdown().await;
        beta.shutdown().await;
    }

    #[tokio::test]
    async fn adopts_a_peer_index_after_legacy_peers_created_separate_documents() {
        let alpha = Peer::start().await;
        let mut beta = Peer::start().await;
        let alpha_capability = alpha.index.join("general", None, vec![]).await.unwrap();
        let beta_capability = beta.index.join("general", None, vec![]).await.unwrap();
        assert_ne!(alpha_capability.raw(), beta_capability.raw());

        let adopted = beta
            .index
            .adopt(
                "general",
                alpha_capability.clone(),
                vec![alpha.router.endpoint().addr()],
            )
            .await
            .unwrap();
        assert_eq!(adopted.raw(), alpha_capability.raw());
        assert_eq!(
            beta.index.capability_for("general").unwrap().raw(),
            alpha_capability.raw()
        );

        alpha
            .index
            .set(
                "general",
                "report.txt".into(),
                postcard::to_stdvec(&FileIndexEntry {
                    content_hash: Hash::new(b"report"),
                    size: 6,
                    modified_unix_ms: 0,
                    author: *alpha.router.endpoint().id().as_bytes(),
                })
                .unwrap(),
            )
            .unwrap();
        let remote = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(NetEvent::RemoteIndex(entry)) = beta.events.recv().await {
                    break entry;
                }
            }
        })
        .await
        .expect("adopted index did not receive a remote entry");
        assert_eq!(remote.channel, "general");
        assert_eq!(remote.path, "report.txt");

        alpha.shutdown().await;
        beta.shutdown().await;
    }
}
