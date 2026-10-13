//! Executable LEY-26.3 spike for the mutable shared-folder index substrate.

use std::time::Duration;

use iroh::{Endpoint, endpoint::presets, protocol::Router};
use iroh_blobs::{BlobsProtocol, Hash, store::fs::FsStore};
use iroh_docs::{
    api::protocol::{AddrInfoOptions, ShareMode},
    protocol::Docs,
    store::Query,
};
use iroh_gossip::net::Gossip;

struct Peer {
    router: Router,
    docs: Docs,
    _data: Option<tempfile::TempDir>,
}

#[tokio::test]
async fn docs_reconcile_changes_made_while_a_persistent_peer_is_offline() {
    let alpha = Peer::start().await;
    let beta_data = tempfile::tempdir().unwrap();
    let beta = Peer::start_at(beta_data.path(), None).await;
    let alpha_author = alpha.docs.author_default().await.unwrap();
    let alpha_doc = alpha.docs.create().await.unwrap();
    let ticket = alpha_doc
        .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
        .await
        .unwrap();
    let _beta_doc = beta.docs.import(ticket.clone()).await.unwrap();
    let key = b"files/offline.txt";

    beta.shutdown().await;
    let added = alpha_doc
        .set_bytes(
            alpha_author,
            key.to_vec(),
            b"written while offline".to_vec(),
        )
        .await
        .unwrap();

    let beta = Peer::start_at(beta_data.path(), None).await;
    let beta_doc = beta.docs.import(ticket).await.unwrap();
    eventual_hash(&beta_doc, key, Some(added)).await;

    alpha.shutdown().await;
    beta.shutdown().await;
}

impl Peer {
    async fn start() -> Self {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().to_path_buf();
        Self::start_at(&path, Some(data)).await
    }

    async fn start_at(path: &std::path::Path, data: Option<tempfile::TempDir>) -> Self {
        let endpoint = Endpoint::bind(presets::Minimal).await.unwrap();
        let blobs = FsStore::load(path.join("blobs")).await.unwrap();
        let gossip = Gossip::builder().spawn(endpoint.clone());
        let docs_dir = path.join("docs");
        std::fs::create_dir_all(&docs_dir).unwrap();
        let docs = Docs::persistent(docs_dir)
            .spawn(endpoint.clone(), blobs.clone().into(), gossip.clone())
            .await
            .unwrap();
        let router = Router::builder(endpoint)
            .accept(iroh_blobs::ALPN, BlobsProtocol::new(&blobs, None))
            .accept(iroh_gossip::net::GOSSIP_ALPN, gossip)
            .accept(iroh_docs::ALPN, docs.clone())
            .spawn();
        Self {
            router,
            docs,
            _data: data,
        }
    }

    async fn shutdown(self) {
        self.router.shutdown().await.unwrap();
    }
}

async fn eventual_hash(doc: &iroh_docs::api::Doc, key: &[u8], expected: Option<Hash>) {
    for _ in 0..100 {
        let entry = doc
            .get_one(Query::single_latest_per_key().key_exact(key))
            .await
            .unwrap();
        if entry.as_ref().map(|entry| entry.content_hash()) == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!(
        "document did not converge for {}",
        String::from_utf8_lossy(key)
    );
}

#[tokio::test]
async fn docs_reconcile_add_edit_and_delete_between_persistent_peers() {
    let alpha = Peer::start().await;
    let beta = Peer::start().await;
    let alpha_author = alpha.docs.author_default().await.unwrap();
    let beta_author = beta.docs.author_default().await.unwrap();
    let alpha_doc = alpha.docs.create().await.unwrap();
    let ticket = alpha_doc
        .share(ShareMode::Write, AddrInfoOptions::RelayAndAddresses)
        .await
        .unwrap();
    let beta_doc = beta.docs.import(ticket).await.unwrap();
    let key = b"files/report.txt";

    let added = alpha_doc
        .set_bytes(alpha_author, key.to_vec(), b"first".to_vec())
        .await
        .unwrap();
    eventual_hash(&beta_doc, key, Some(added)).await;

    let edited = beta_doc
        .set_bytes(beta_author, key.to_vec(), b"second".to_vec())
        .await
        .unwrap();
    eventual_hash(&alpha_doc, key, Some(edited)).await;

    beta_doc.del(beta_author, key.to_vec()).await.unwrap();
    eventual_hash(&alpha_doc, key, None).await;
    eventual_hash(&beta_doc, key, None).await;

    alpha.shutdown().await;
    beta.shutdown().await;
}
