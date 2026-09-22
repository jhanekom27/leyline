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
use iroh_gossip::{
    api::{Event as GossipEvent, GossipReceiver, GossipSender},
    net::{GOSSIP_ALPN, Gossip},
    proto::TopicId,
};
use iroh_tickets::{ParseError, Ticket};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::message::ChatMessage;
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
    /// A chat message was received and decoded.
    Received(String, ChatMessage),
    /// The receiver missed some messages because it wasn't keeping up.
    Lagged(String),
    /// A channel finished subscribing and is ready to send/receive on.
    Joined(String),
    /// A channel failed to join.
    JoinFailed(String, String),
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
    /// Our own endpoint id, as raw bytes so callers don't need to depend on
    /// iroh types.
    pub our_id: [u8; 32],
}

impl Net {
    /// Binds an endpoint using `secret_key`, joins the default "general"
    /// channel (dialing the bootstrap peer from `join_ticket` if it names
    /// "general"), additionally joins `join_ticket`'s channel if it names
    /// something else, and spawns a gossip task per joined channel
    /// forwarding events to `events_tx`. Returns the names of the channels
    /// joined, in join order, so the caller can decide e.g. which one
    /// starts active.
    pub async fn start(
        secret_key: SecretKey,
        join_ticket: Option<String>,
        events_tx: mpsc::Sender<NetEvent>,
    ) -> anyhow::Result<(Self, Vec<String>)> {
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
        info!(%id, "endpoint bound");

        // Wait for a home relay so the address used for our own tickets
        // below has a usable relay hint, not just a bare id.
        endpoint.online().await;

        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = Router::builder(endpoint)
            .accept(GOSSIP_ALPN, gossip.clone())
            .spawn();

        let net = Self {
            router,
            gossip,
            address_lookup,
            senders: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            our_id: *id.as_bytes(),
        };

        let general_bootstrap = ticket
            .as_ref()
            .filter(|t| t.name == DEFAULT_CHANNEL)
            .map(|t| t.addr.clone());
        net.subscribe_and_register(DEFAULT_CHANNEL, general_bootstrap)
            .await
            .context("failed to join default channel")?;
        let mut joined = vec![DEFAULT_CHANNEL.to_string()];

        if let Some(ticket) = ticket
            && ticket.name != DEFAULT_CHANNEL
        {
            net.subscribe_and_register(&ticket.name, Some(ticket.addr))
                .await
                .with_context(|| format!("failed to join channel {:?}", ticket.name))?;
            joined.push(ticket.name);
        }

        Ok((net, joined))
    }

    /// Subscribes to `name`'s topic, registers the resulting sender, and
    /// spawns its forwarding (and, if `bootstrap` is given, retry-join)
    /// tasks. Shared by startup (`Net::start`) and runtime (`Net::join`)
    /// joins so the two paths can't drift apart.
    async fn subscribe_and_register(
        &self,
        name: &str,
        bootstrap: Option<EndpointAddr>,
    ) -> anyhow::Result<()> {
        if let Some(addr) = &bootstrap {
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
        ));

        Ok(())
    }

    /// Joins (or creates) a channel named by a bare name or an invite
    /// ticket string. Returns `Some(message)` immediately if `arg` looks
    /// like a ticket but fails to decode (see `parse_join_arg`) -- this is
    /// checked synchronously so the caller can show it right away. On
    /// success, this is otherwise fire-and-forget: spawns its own task and
    /// reports the outcome back as a `NetEvent::Joined`/`JoinFailed`, so
    /// the caller's event loop never awaits network I/O directly.
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

        let gossip = self.gossip.clone();
        let address_lookup = self.address_lookup.clone();
        let senders = Arc::clone(&self.senders);
        let events_tx = self.events_tx.clone();
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
                    tokio::spawn(forward_events(name.clone(), receiver, events_tx.clone()));
                    let _ = events_tx.send(NetEvent::Joined(name)).await;
                }
                Err(err) => {
                    let _ = events_tx
                        .send(NetEvent::JoinFailed(name, err.to_string()))
                        .await;
                }
            }
        });
        None
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
/// dropped.
async fn forward_events(
    channel: String,
    mut receiver: GossipReceiver,
    events_tx: mpsc::Sender<NetEvent>,
) {
    while let Some(event) = receiver.next().await {
        let net_event = match event {
            Ok(GossipEvent::NeighborUp(id)) => {
                NetEvent::PeerJoined(channel.clone(), *id.as_bytes())
            }
            Ok(GossipEvent::NeighborDown(id)) => {
                NetEvent::PeerLeft(channel.clone(), *id.as_bytes())
            }
            Ok(GossipEvent::Received(msg)) => {
                match postcard::from_bytes::<ChatMessage>(&msg.content) {
                    Ok(message) => NetEvent::Received(channel.clone(), message),
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
}
