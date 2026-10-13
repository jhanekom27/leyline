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
use iroh_blobs::store::fs::FsStore;
use iroh_docs::{
    AuthorId, Capability,
    api::Doc,
    api::protocol::{AddrInfoOptions, ShareMode},
    protocol::Docs,
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
        while let Some(event) = stream.next().await {
            match event {
                Ok(iroh_docs::engine::LiveEvent::InsertRemote { entry, .. }) => {
                    let Ok(path) = String::from_utf8(entry.key().to_vec()) else {
                        warn!(%channel, "dropping non-UTF-8 remote index path");
                        continue;
                    };
                    let metadata = match blobs.blobs().get_bytes(entry.content_hash()).await {
                        Ok(bytes) => bytes,
                        Err(err) => {
                            warn!(%channel, "failed to read remote index metadata: {err}");
                            continue;
                        }
                    };
                    let entry = match postcard::from_bytes::<FileIndexEntry>(&metadata) {
                        Ok(entry) => entry,
                        Err(err) => {
                            warn!(%channel, "dropping malformed remote index metadata: {err}");
                            continue;
                        }
                    };
                    if events_tx
                        .send(NetEvent::RemoteIndex(RemoteIndexEntry {
                            channel: channel.clone(),
                            path,
                            entry,
                        }))
                        .await
                        .is_err()
                    {
                        break;
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
