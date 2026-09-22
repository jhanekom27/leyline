//! iroh + iroh-gossip networking.
//!
//! Owns an `iroh::Endpoint` and a subscription to the single default gossip
//! topic (multi-channel support is build-order step 4), forwarding parsed
//! `NetEvent`s into the app's event loop over an `mpsc` channel -- see
//! concept.md's "Architecture at a glance" section.

use std::time::Duration;

use anyhow::Context;
use futures::StreamExt;
use iroh::{
    Endpoint, EndpointId, SecretKey, address_lookup::memory::MemoryLookup, endpoint::presets,
    protocol::Router,
};
use iroh_gossip::{
    api::{Event as GossipEvent, GossipReceiver, GossipSender},
    net::{GOSSIP_ALPN, Gossip},
    proto::TopicId,
};
use iroh_tickets::Ticket;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::message::ChatMessage;
use crate::ticket::ChannelTicket;

/// How many times to retry joining the ticket's bootstrap peer, and how long
/// to wait between attempts.
///
/// The ticket carries the bootstrap peer's address hints directly, so the
/// first join attempt shouldn't need discovery at all -- but the peer could
/// still be transiently unreachable (e.g. offline, or its address changed
/// since the ticket was created), and iroh-gossip does not retry a
/// bootstrap peer on its own if the first dial fails. Re-issuing the join a
/// few times covers that.
const JOIN_RETRY_ATTEMPTS: u32 = 5;
const JOIN_RETRY_INTERVAL: Duration = Duration::from_secs(3);

/// Events forwarded from the network into the app's event loop.
pub enum NetEvent {
    /// A peer joined the topic's mesh as a direct neighbor.
    PeerJoined([u8; 32]),
    /// A peer dropped from the topic's mesh.
    PeerLeft([u8; 32]),
    /// A chat message was received and decoded.
    Received(ChatMessage),
    /// The receiver missed some messages because it wasn't keeping up.
    Lagged,
}

/// The single default channel's topic, until multi-channel support
/// (build-order step 4) lands. Derived deterministically so every instance
/// joins the same swarm -- see concept.md's "Identity & channels" section.
fn default_topic() -> TopicId {
    blake3::hash(b"leyline:general").into()
}

/// Handle to the running network stack.
///
/// Keeps the endpoint/router alive (dropping the router aborts its accept
/// loop) and exposes a way to broadcast chat messages.
pub struct Net {
    router: Router,
    sender: GossipSender,
    /// Our own endpoint id, as raw bytes so callers don't need to depend on
    /// iroh types.
    pub our_id: [u8; 32],
    /// This instance's invite ticket (default channel topic + our address),
    /// pre-encoded to its base32 string form for `main.rs` to print.
    pub ticket: String,
}

impl Net {
    /// Binds an endpoint using `secret_key`, joins the default gossip topic
    /// (dialing the bootstrap peer from `join_ticket` if given), and spawns
    /// a task forwarding gossip events to `events_tx`. Joining a bootstrap
    /// peer happens in the background, so this never blocks on it being
    /// reachable.
    pub async fn start(
        secret_key: SecretKey,
        join_ticket: Option<String>,
        events_tx: mpsc::Sender<NetEvent>,
    ) -> anyhow::Result<Self> {
        let bootstrap = join_ticket
            .map(|raw| ChannelTicket::decode_string(&raw))
            .transpose()
            .context("invalid --join ticket")?;
        if let Some(ticket) = &bootstrap {
            anyhow::ensure!(
                ticket.topic == default_topic(),
                "that ticket is for a different channel; multi-channel support isn't available yet"
            );
        }
        let bootstrap_addr = bootstrap.map(|ticket| ticket.addr);

        // Pre-seed the endpoint's address book with the ticket's address
        // hints, so the bootstrap peer is immediately dialable instead of
        // waiting on the (still separately configured) pkarr/DNS discovery
        // in `presets::N0` to resolve it.
        let memory_lookup = MemoryLookup::new();
        if let Some(addr) = &bootstrap_addr {
            memory_lookup.add_endpoint_info(addr.clone());
        }

        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(secret_key)
            .address_lookup(memory_lookup)
            .bind()
            .await
            .context("failed to bind iroh endpoint")?;
        let id = endpoint.id();
        info!(%id, "endpoint bound");

        // Wait for a home relay so the `endpoint.addr()` used for our own
        // ticket below has a usable relay hint, not just a bare id.
        endpoint.online().await;

        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = Router::builder(endpoint.clone())
            .accept(GOSSIP_ALPN, gossip.clone())
            .spawn();

        let bootstrap_ids: Vec<EndpointId> = bootstrap_addr.iter().map(|addr| addr.id).collect();
        let (sender, receiver) = gossip
            .subscribe(default_topic(), bootstrap_ids.clone())
            .await
            .context("failed to subscribe to gossip topic")?
            .split();
        debug!(bootstrap = bootstrap_ids.len(), "joined default topic");

        if !bootstrap_ids.is_empty() {
            tokio::spawn(retry_join(sender.clone(), bootstrap_ids));
        }
        tokio::spawn(forward_events(receiver, events_tx));

        let ticket = ChannelTicket {
            topic: default_topic(),
            addr: endpoint.addr(),
        }
        .encode_string();

        Ok(Self {
            router,
            sender,
            our_id: *id.as_bytes(),
            ticket,
        })
    }

    /// Encodes and broadcasts a chat message to the topic. Fire-and-forget:
    /// spawns its own task so the caller's event loop never awaits network
    /// I/O directly.
    pub fn send(&self, message: ChatMessage) {
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let bytes = match postcard::to_stdvec(&message) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("failed to encode chat message: {err}");
                    return;
                }
            };
            if let Err(err) = sender.broadcast(bytes.into()).await {
                warn!("failed to broadcast chat message: {err}");
            }
        });
    }

    /// Leaves the topic and closes the endpoint gracefully.
    pub async fn shutdown(&self) -> anyhow::Result<()> {
        self.router
            .shutdown()
            .await
            .context("failed to shut down router")
    }
}

/// Periodically re-sends a join request for `bootstrap`, since iroh-gossip
/// does not retry a bootstrap peer on its own if the first dial to it fails
/// or is still resolving via discovery. Harmless once already connected --
/// re-joining an active peer is a no-op for the underlying protocol.
async fn retry_join(sender: GossipSender, bootstrap: Vec<EndpointId>) {
    for attempt in 1..=JOIN_RETRY_ATTEMPTS {
        tokio::time::sleep(JOIN_RETRY_INTERVAL).await;
        debug!(attempt, "retrying join of bootstrap peers");
        if let Err(err) = sender.join_peers(bootstrap.clone()).await {
            warn!("failed to retry joining bootstrap peers: {err}");
            return;
        }
    }
}

/// Reads gossip events off `receiver` and forwards them as `NetEvent`s until
/// the receiver closes or `events_tx` is dropped.
async fn forward_events(mut receiver: GossipReceiver, events_tx: mpsc::Sender<NetEvent>) {
    while let Some(event) = receiver.next().await {
        let net_event = match event {
            Ok(GossipEvent::NeighborUp(id)) => NetEvent::PeerJoined(*id.as_bytes()),
            Ok(GossipEvent::NeighborDown(id)) => NetEvent::PeerLeft(*id.as_bytes()),
            Ok(GossipEvent::Received(msg)) => {
                match postcard::from_bytes::<ChatMessage>(&msg.content) {
                    Ok(message) => NetEvent::Received(message),
                    Err(err) => {
                        warn!("dropping malformed gossip message: {err}");
                        continue;
                    }
                }
            }
            Ok(GossipEvent::Lagged) => {
                warn!("gossip receiver lagged; some messages may have been missed");
                NetEvent::Lagged
            }
            Err(err) => {
                warn!("gossip receiver closed: {err}");
                break;
            }
        };
        if events_tx.send(net_event).await.is_err() {
            break;
        }
    }
    debug!("gossip event forwarder stopped");
}
