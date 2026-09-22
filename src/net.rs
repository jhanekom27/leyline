//! iroh + iroh-gossip networking.
//!
//! Owns an `iroh::Endpoint` and a subscription per joined gossip topic --
//! one gossip task per channel, per concept.md's "Architecture at a glance"
//! section -- forwarding parsed `NetEvent`s, each tagged with the channel
//! name it came from, into the app's event loop over an `mpsc` channel.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use futures::StreamExt;
use iroh::{
    Endpoint, EndpointAddr, EndpointId, SecretKey, address_lookup::memory::MemoryLookup,
    endpoint::presets, protocol::Router,
};
use iroh_blobs::Hash;
use iroh_gossip::{
    api::{Event as GossipEvent, GossipReceiver, GossipSender},
    net::{GOSSIP_ALPN, Gossip},
    proto::TopicId,
};
use iroh_tickets::{ParseError, Ticket};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::backfill::BackfillStore;
use crate::message::{ChatMessage, GossipPayload, HistoryAnnounce};
use crate::ticket::ChannelTicket;

/// The channel every instance joins on startup.
const DEFAULT_CHANNEL: &str = "general";

/// How many times to retry joining a channel's bootstrap peer, and how long
/// to wait between attempts.
///
/// A ticket carries the bootstrap peer's address hints directly, so the
/// first join attempt shouldn't need discovery at all -- but the peer could
/// still be transiently unreachable (e.g. offline, or its address changed
/// since the ticket was created), and iroh-gossip does not retry a
/// bootstrap peer on its own if the first dial fails. Re-issuing the join a
/// few times covers that.
const JOIN_RETRY_ATTEMPTS: u32 = 5;
const JOIN_RETRY_INTERVAL: Duration = Duration::from_secs(3);

/// Events forwarded from the network into the app's event loop, each tagged
/// with the name of the channel it concerns.
pub enum NetEvent {
    /// A peer joined a channel's mesh as a direct neighbor.
    PeerJoined(String, [u8; 32]),
    /// A peer dropped from a channel's mesh.
    PeerLeft(String, [u8; 32]),
    /// A gossip neighbor's dialable address (relay/direct-address hints)
    /// was resolved via `Endpoint::remote_info`, ready to persist as a
    /// bootstrap hint for reconnecting to this channel on a future run --
    /// see `crate::channel_registry`. Best-effort: not every `PeerJoined`
    /// produces one of these, since `remote_info` can return `None`.
    PeerAddressLearned(String, EndpointAddr),
    /// A chat message was received and decoded.
    Received(String, ChatMessage),
    /// The receiver missed some messages because it wasn't keeping up.
    Lagged(String),
    /// A channel finished subscribing and is ready to send/receive on.
    Joined(String),
    /// A channel failed to join.
    JoinFailed(String, String),
    /// A peer announced its current history root hash for a channel -- see
    /// `crate::message::HistoryAnnounce` and backfill.rs.
    Announce(String, HistoryAnnounce),
    /// A history backfill fetch finished and decoded some messages, ready
    /// to be deduped, persisted, and merged into the transcript.
    HistoryFetched(String, Vec<ChatMessage>),
}

/// A channel's gossip topic is a deterministic hash of its human-readable
/// name, so friends can agree on a channel out of band without a directory
/// service -- see concept.md's "Identity & channels" section. Channel
/// identity is threaded through the rest of the app as this plain name;
/// only `net.rs` needs to know about `TopicId`.
fn topic_for_name(name: &str) -> TopicId {
    blake3::hash(format!("leyline:{name}").as_bytes()).into()
}

/// Interprets a `/join` argument as either a bare channel name or an
/// invite ticket.
///
/// If `arg` doesn't even have the ticket prefix, it's treated as a plain
/// channel name. If it *does* have the prefix but still fails to decode --
/// e.g. because it was truncated when copied out of a narrow terminal --
/// that's reported as an error instead of silently joining a bogus channel
/// named after the mangled string.
fn parse_join_arg(arg: &str) -> Result<(String, Option<EndpointAddr>), String> {
    match ChannelTicket::decode_string(arg) {
        Ok(ticket) => Ok((ticket.name, Some(ticket.addr))),
        Err(ParseError::Kind { .. }) => Ok((arg.trim().to_string(), None)),
        Err(err) => Err(format!(
            "that looks like an invite ticket but it won't decode (maybe it got truncated when copied?): {err}"
        )),
    }
}

/// `true` if `addr` names our own endpoint id.
///
/// Used to detect a self-referential invite ticket -- e.g. pasting your
/// own `/invite` ticket back in after a restart, since a ticket always
/// encodes the *sharer's* address (`Net::ticket_for`), not a durable
/// channel meeting point. iroh treats connecting to yourself as a no-op,
/// so keeping such an address as a bootstrap candidate would otherwise
/// produce a silently isolated topic subscription instead of a clear
/// outcome.
fn is_self(addr: &EndpointAddr, our_id: [u8; 32]) -> bool {
    *addr.id.as_bytes() == our_id
}

/// Handle to the running network stack.
///
/// Keeps the endpoint/router alive (dropping the router aborts its accept
/// loop) and exposes ways to join additional channels and broadcast chat
/// messages.
pub struct Net {
    router: Router,
    gossip: Gossip,
    /// Address hints for peers we've been given tickets for, so bootstrap
    /// peers are immediately dialable instead of waiting on discovery.
    /// Retained (rather than handed to the endpoint builder and dropped) so
    /// later joins can add more hints the same way.
    address_lookup: MemoryLookup,
    /// One gossip sender per joined channel, keyed by topic. Shared with
    /// spawned join tasks, which insert their new sender here once
    /// subscribed -- see `Net::join`.
    senders: Arc<Mutex<HashMap<TopicId, GossipSender>>>,
    events_tx: mpsc::Sender<NetEvent>,
    /// Backs history backfill (see backfill.rs): serves our blobs to peers
    /// that fetch from us, and lets us fetch from a peer that announced a
    /// root hash we don't have yet.
    backfill: BackfillStore,
    /// Our own endpoint id, as raw bytes so callers don't need to depend on
    /// iroh types.
    pub our_id: [u8; 32],
}

impl Net {
    /// Binds an endpoint using `secret_key`, then joins the default
    /// "general" channel plus every channel in `known_channels` (the
    /// previous session's `ChannelRegistry`, so nothing joined last time is
    /// forgotten -- see `crate::channel_registry`), seeding bootstrap peers
    /// from each channel's stored addresses. If `join_ticket` names a
    /// channel not already covered, that channel is joined too, using the
    /// ticket's address as an extra bootstrap candidate. Spawns a gossip
    /// task per joined channel forwarding events to `events_tx`. Returns
    /// the names of the channels joined, in join order, plus the name of
    /// the channel that should start active (the `--join` ticket's
    /// channel, if one was given and usable, else "general").
    pub async fn start(
        secret_key: SecretKey,
        join_ticket: Option<String>,
        known_channels: Vec<(String, Vec<EndpointAddr>)>,
        events_tx: mpsc::Sender<NetEvent>,
        backfill: BackfillStore,
    ) -> anyhow::Result<(Self, Vec<String>, String)> {
        let ticket = join_ticket
            .map(|raw| ChannelTicket::decode_string(&raw))
            .transpose()
            .context("invalid --join ticket")?;

        // Pre-seed the endpoint's address book with the ticket's address
        // hints, so the bootstrap peer is immediately dialable instead of
        // waiting on the (still separately configured) pkarr/DNS discovery
        // in `presets::N0` to resolve it.
        let address_lookup = MemoryLookup::new();
        if let Some(ticket) = &ticket {
            address_lookup.add_endpoint_info(ticket.addr.clone());
        }

        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(secret_key)
            .address_lookup(address_lookup.clone())
            .bind()
            .await
            .context("failed to bind iroh endpoint")?;
        let id = endpoint.id();
        let our_id = *id.as_bytes();
        info!(%id, "endpoint bound");

        // Wait for a home relay so the address used for our own tickets
        // below has a usable relay hint, not just a bare id.
        endpoint.online().await;

        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = Router::builder(endpoint)
            .accept(GOSSIP_ALPN, gossip.clone())
            .accept(iroh_blobs::ALPN, backfill.protocol_handler())
            .spawn();

        let net = Self {
            router,
            gossip,
            address_lookup,
            senders: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            backfill,
            our_id,
        };

        // A ticket naming our own identity can't reach anyone -- an invite
        // ticket always encodes the *sharer's* address (see
        // `Net::ticket_for`), so pasting your own back in (e.g. after a
        // restart) would otherwise silently try to bootstrap to yourself.
        // Drop it as a bootstrap candidate; any peers already known for
        // that channel (from `known_channels`) are used instead.
        let ticket = ticket.filter(|t| {
            let usable = !is_self(&t.addr, our_id);
            if !usable {
                warn!(
                    channel = %t.name,
                    "--join ticket points at our own identity; ignoring it and using known peers instead"
                );
            }
            usable
        });

        let mut joined = Vec::new();

        let mut general_bootstrap = known_channels
            .iter()
            .find(|(name, _)| name.as_str() == DEFAULT_CHANNEL)
            .map(|(_, addrs)| addrs.clone())
            .unwrap_or_default();
        if let Some(t) = ticket.as_ref().filter(|t| t.name == DEFAULT_CHANNEL) {
            general_bootstrap.push(t.addr.clone());
        }
        net.subscribe_and_register(DEFAULT_CHANNEL, general_bootstrap)
            .await
            .context("failed to join default channel")?;
        joined.push(DEFAULT_CHANNEL.to_string());

        for (name, addrs) in &known_channels {
            if name.as_str() == DEFAULT_CHANNEL {
                continue;
            }
            let mut bootstrap = addrs.clone();
            if let Some(t) = ticket.as_ref().filter(|t| t.name.as_str() == name.as_str()) {
                bootstrap.push(t.addr.clone());
            }
            net.subscribe_and_register(name, bootstrap)
                .await
                .with_context(|| format!("failed to rejoin channel {name:?}"))?;
            joined.push(name.clone());
        }

        let active = match &ticket {
            Some(t) if !joined.contains(&t.name) => {
                net.subscribe_and_register(&t.name, vec![t.addr.clone()])
                    .await
                    .with_context(|| format!("failed to join channel {:?}", t.name))?;
                joined.push(t.name.clone());
                t.name.clone()
            }
            Some(t) => t.name.clone(),
            None => DEFAULT_CHANNEL.to_string(),
        };

        Ok((net, joined, active))
    }

    /// Subscribes to `name`'s topic, registers the resulting sender, and
    /// spawns its forwarding (and, if `bootstrap` is non-empty, retry-join)
    /// tasks. Shared by startup (`Net::start`) and runtime (`Net::join`)
    /// joins so the two paths can't drift apart. `bootstrap` may combine
    /// addresses from more than one source (e.g. a `--join` ticket and
    /// previously-learned peers from `crate::channel_registry`) --
    /// duplicates are harmless, since iroh-gossip dedupes its bootstrap set
    /// internally.
    async fn subscribe_and_register(
        &self,
        name: &str,
        bootstrap: Vec<EndpointAddr>,
    ) -> anyhow::Result<()> {
        for addr in &bootstrap {
            self.address_lookup.add_endpoint_info(addr.clone());
        }
        let bootstrap_ids: Vec<EndpointId> = bootstrap.iter().map(|addr| addr.id).collect();

        let topic = topic_for_name(name);
        let (sender, receiver) = self
            .gossip
            .subscribe(topic, bootstrap_ids.clone())
            .await
            .context("failed to subscribe to gossip topic")?
            .split();
        debug!(channel = %name, bootstrap = bootstrap_ids.len(), "joined channel");

        self.senders
            .lock()
            .expect("senders lock poisoned")
            .insert(topic, sender.clone());

        if !bootstrap_ids.is_empty() {
            tokio::spawn(retry_join(sender, bootstrap_ids));
        }
        tokio::spawn(forward_events(
            name.to_string(),
            receiver,
            self.events_tx.clone(),
            self.router.endpoint().clone(),
        ));

        Ok(())
    }

    /// Joins (or creates) a channel named by a bare name or an invite
    /// ticket string. Returns `Some(message)` immediately if `arg` looks
    /// like a ticket but fails to decode (see `parse_join_arg`) -- this is
    /// checked synchronously so the caller can show it right away. Also
    /// returns `Some(message)` if the ticket decodes but names our own
    /// identity (see `is_self`); unlike the decode-failure case, the join
    /// below still proceeds (using no bootstrap, since a self-referential
    /// one is useless) so the channel is created/switched to regardless.
    /// Otherwise this is fire-and-forget: spawns its own task and reports
    /// the outcome back as a `NetEvent::Joined`/`JoinFailed`, so the
    /// caller's event loop never awaits network I/O directly.
    ///
    /// A no-op re-subscribe if the channel is already joined -- it still
    /// reports `Joined` so the UI can switch to it.
    pub fn join(&self, arg: String) -> Option<String> {
        let (name, bootstrap) = match parse_join_arg(&arg) {
            Ok(parsed) => parsed,
            Err(error) => return Some(error),
        };
        if name.is_empty() {
            return None;
        }

        let topic = topic_for_name(&name);
        if self
            .senders
            .lock()
            .expect("senders lock poisoned")
            .contains_key(&topic)
        {
            let _ = self.events_tx.try_send(NetEvent::Joined(name));
            return None;
        }

        let self_ticket = bootstrap
            .as_ref()
            .is_some_and(|addr| is_self(addr, self.our_id));
        let bootstrap = if self_ticket { None } else { bootstrap };
        let message = self_ticket.then(|| {
            format!(
                "that invite ticket points at your own identity, so it can't connect you to anyone -- joining #{name} anyway"
            )
        });

        let gossip = self.gossip.clone();
        let address_lookup = self.address_lookup.clone();
        let senders = Arc::clone(&self.senders);
        let events_tx = self.events_tx.clone();
        let endpoint = self.router.endpoint().clone();
        tokio::spawn(async move {
            if let Some(addr) = &bootstrap {
                address_lookup.add_endpoint_info(addr.clone());
            }
            let bootstrap_ids: Vec<EndpointId> = bootstrap.iter().map(|addr| addr.id).collect();

            match gossip.subscribe(topic, bootstrap_ids.clone()).await {
                Ok(topic_handle) => {
                    let (sender, receiver) = topic_handle.split();
                    senders
                        .lock()
                        .expect("senders lock poisoned")
                        .insert(topic, sender.clone());
                    if !bootstrap_ids.is_empty() {
                        tokio::spawn(retry_join(sender, bootstrap_ids));
                    }
                    tokio::spawn(forward_events(
                        name.clone(),
                        receiver,
                        events_tx.clone(),
                        endpoint,
                    ));
                    let _ = events_tx.send(NetEvent::Joined(name)).await;
                }
                Err(err) => {
                    let _ = events_tx
                        .send(NetEvent::JoinFailed(name, err.to_string()))
                        .await;
                }
            }
        });

        message
    }

    /// Encodes and broadcasts a chat message to `channel`. Fire-and-forget:
    /// spawns its own task so the caller's event loop never awaits network
    /// I/O directly. Drops the message if `channel` isn't joined -- the UI
    /// shouldn't be able to compose a message for a channel it doesn't know
    /// about, so this would indicate a bug elsewhere.
    pub fn send(&self, channel: &str, message: ChatMessage) {
        let topic = topic_for_name(channel);
        let Some(sender) = self
            .senders
            .lock()
            .expect("senders lock poisoned")
            .get(&topic)
            .cloned()
        else {
            warn!(%channel, "dropping message for unjoined channel");
            return;
        };
        tokio::spawn(async move {
            let bytes = match postcard::to_stdvec(&GossipPayload::Chat(message)) {
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

    /// Announces our current history root hash for `channel` to our direct
    /// gossip neighbors (see backfill.rs) -- sent whenever a channel gains a
    /// neighbor, in either direction, so both a fresh join and a reconnect
    /// give both sides a chance to notice they're missing something. Uses
    /// `broadcast_neighbors` rather than a full-mesh `broadcast`, since this
    /// is inherently a neighbor-to-neighbor concern, not something that
    /// needs flooding. Fire-and-forget, like `send`.
    pub fn announce(&self, channel: &str, root: Hash) {
        let topic = topic_for_name(channel);
        let Some(sender) = self
            .senders
            .lock()
            .expect("senders lock poisoned")
            .get(&topic)
            .cloned()
        else {
            return;
        };
        let payload = GossipPayload::Announce(HistoryAnnounce {
            sender: self.our_id,
            root,
        });
        tokio::spawn(async move {
            let bytes = match postcard::to_stdvec(&payload) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("failed to encode history announce: {err}");
                    return;
                }
            };
            if let Err(err) = sender.broadcast_neighbors(bytes.into()).await {
                warn!("failed to broadcast history announce: {err}");
            }
        });
    }

    /// Reacts to a received `HistoryAnnounce`: if its root differs from what
    /// we already have for `channel` (and it isn't an echo of our own
    /// announce), fetches and decodes the delta directly from the
    /// announcing peer and reports it back as `NetEvent::HistoryFetched`.
    /// Fire-and-forget, like `send` and `join` -- a failed fetch is logged
    /// and simply retried on the next announce, matching how presence and
    /// other gossip state elsewhere is treated as eventually consistent
    /// rather than a source of truth.
    pub fn sync_history(&self, channel: String, announce: HistoryAnnounce) {
        if announce.sender == self.our_id {
            return;
        }
        if self.backfill.current_root(&channel) == Some(announce.root) {
            return;
        }

        let backfill = self.backfill.clone();
        let endpoint = self.router.endpoint().clone();
        let events_tx = self.events_tx.clone();
        tokio::spawn(async move {
            match backfill
                .fetch(&endpoint, &channel, announce.root, announce.sender)
                .await
            {
                Ok(messages) if !messages.is_empty() => {
                    let _ = events_tx
                        .send(NetEvent::HistoryFetched(channel, messages))
                        .await;
                }
                Ok(_) => {}
                Err(err) => warn!(%channel, "failed to sync history: {err}"),
            }
        });
    }

    /// Builds this instance's invite ticket string for `channel`: our
    /// current address plus the channel's name, ready to be pasted into
    /// another instance's `--join` flag or `/join` command. Synchronous --
    /// `Endpoint::addr` doesn't need to await anything.
    pub fn ticket_for(&self, channel: &str) -> String {
        ChannelTicket {
            name: channel.to_string(),
            addr: self.router.endpoint().addr(),
        }
        .encode_string()
    }

    /// Leaves every channel and closes the endpoint gracefully.
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

/// Reads gossip events off `receiver` and forwards them as `NetEvent`s
/// tagged with `channel` until the receiver closes or `events_tx` is
/// dropped. `endpoint` is used to resolve a newly-up neighbor's dialable
/// address (see `announce_learned_address`).
async fn forward_events(
    channel: String,
    mut receiver: GossipReceiver,
    events_tx: mpsc::Sender<NetEvent>,
    endpoint: Endpoint,
) {
    while let Some(event) = receiver.next().await {
        let net_event = match event {
            Ok(GossipEvent::NeighborUp(id)) => {
                announce_learned_address(&channel, id, &endpoint, &events_tx).await;
                NetEvent::PeerJoined(channel.clone(), *id.as_bytes())
            }
            Ok(GossipEvent::NeighborDown(id)) => {
                NetEvent::PeerLeft(channel.clone(), *id.as_bytes())
            }
            Ok(GossipEvent::Received(msg)) => {
                match postcard::from_bytes::<GossipPayload>(&msg.content) {
                    Ok(GossipPayload::Chat(message)) => {
                        NetEvent::Received(channel.clone(), message)
                    }
                    Ok(GossipPayload::Announce(announce)) => {
                        NetEvent::Announce(channel.clone(), announce)
                    }
                    Err(err) => {
                        warn!("dropping malformed gossip message: {err}");
                        continue;
                    }
                }
            }
            Ok(GossipEvent::Lagged) => {
                warn!(%channel, "gossip receiver lagged; some messages may have been missed");
                NetEvent::Lagged(channel.clone())
            }
            Err(err) => {
                warn!(%channel, "gossip receiver closed: {err}");
                break;
            }
        };
        if events_tx.send(net_event).await.is_err() {
            break;
        }
    }
    debug!(%channel, "gossip event forwarder stopped");
}

/// Resolves a newly-up neighbor's dialable address (relay/direct hints)
/// via `Endpoint::remote_info` and reports it as
/// `NetEvent::PeerAddressLearned`, so it can be persisted as a bootstrap
/// hint for a future restart -- see `crate::channel_registry`.
/// Best-effort: `remote_info` can return `None` (e.g. no address info
/// cached for `id` yet), in which case this is simply a no-op; presence
/// (`NetEvent::PeerJoined`) is still reported by the caller regardless.
async fn announce_learned_address(
    channel: &str,
    id: EndpointId,
    endpoint: &Endpoint,
    events_tx: &mpsc::Sender<NetEvent>,
) {
    let Some(info) = endpoint.remote_info(id).await else {
        return;
    };
    let addr = EndpointAddr::from_parts(info.id(), info.into_addrs().map(|a| a.into_addr()));
    let _ = events_tx
        .send(NetEvent::PeerAddressLearned(channel.to_string(), addr))
        .await;
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

    #[test]
    fn parse_join_arg_treats_a_bare_name_as_a_channel_name() {
        let (name, bootstrap) = parse_join_arg("project-x").unwrap();
        assert_eq!(name, "project-x");
        assert!(bootstrap.is_none());
    }

    #[test]
    fn parse_join_arg_decodes_a_valid_ticket() {
        let addr = sample_addr();
        let ticket = ChannelTicket {
            name: "general".to_string(),
            addr: addr.clone(),
        }
        .encode_string();

        let (name, bootstrap) = parse_join_arg(&ticket).unwrap();
        assert_eq!(name, "general");
        assert_eq!(bootstrap, Some(addr));
    }

    #[test]
    fn parse_join_arg_reports_a_truncated_ticket_as_an_error_not_a_name() {
        let ticket = ChannelTicket {
            name: "general".to_string(),
            addr: sample_addr(),
        }
        .encode_string();
        // Simulate a ticket cut off partway through, e.g. by a terminal
        // that clipped a long line before it was copied.
        let truncated = &ticket[..ticket.len() / 2];

        let error = parse_join_arg(truncated).unwrap_err();
        assert!(
            error.contains("invite ticket"),
            "error should explain what went wrong, got: {error}"
        );
    }

    #[test]
    fn parse_join_arg_rejects_an_empty_ticket_prefix_only() {
        // Just the "leyline" prefix with nothing after it: still looks
        // like an attempted ticket, so it should error, not become a
        // channel literally named "leyline".
        let error = parse_join_arg("leyline").unwrap_err();
        assert!(error.contains("invite ticket"));
    }

    #[test]
    fn is_self_detects_an_address_naming_our_own_id() {
        let our_key = SecretKey::generate();
        let our_id = *our_key.public().as_bytes();
        let our_addr = EndpointAddr::from_parts(our_key.public(), []);

        assert!(is_self(&our_addr, our_id));
        assert!(!is_self(&sample_addr(), our_id));
    }
}
