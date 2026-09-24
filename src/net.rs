//! iroh + iroh-gossip networking.
//!
//! Owns an `iroh::Endpoint` and a subscription per joined gossip topic --
//! one gossip task per channel, per concept.md's "Architecture at a glance"
//! section -- forwarding parsed `NetEvent`s, each tagged with the channel
//! name it came from, into the app's event loop over an `mpsc` channel.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use futures::StreamExt;
use iroh::{
    Endpoint, EndpointAddr, EndpointId, NET_REPORT_TIMEOUT, SecretKey,
    address_lookup::memory::MemoryLookup, endpoint::presets, protocol::Router,
};
use iroh_blobs::Hash;
use iroh_gossip::{
    api::{Event as GossipEvent, GossipReceiver, GossipSender},
    net::{GOSSIP_ALPN, Gossip},
    proto::TopicId,
};
use iroh_tickets::{ParseError, Ticket};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::backfill::BackfillStore;
use crate::message::{
    ChatMessage, FileAttachment, GossipPayload, HistoryAnnounce, IdentityAnnounce, now_unix_ms,
};
use crate::ticket::{ChannelTicket, RoomSecret};

/// Simple guardrail against an accidental huge `/send` (e.g. a mistyped
/// path to a large video file): `BackfillStore::add_file` copies the file
/// into the local blob store, so a sent file's bytes are duplicated on
/// disk locally -- see net.rs's `send_file`.
const MAX_FILE_SIZE: u64 = 500 * 1024 * 1024;

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
    /// A peer announced (or re-announced) their broadcast nickname -- see
    /// `crate::message::IdentityAnnounce`.
    Identity(String, IdentityAnnounce),
    /// A history backfill fetch finished and decoded some messages, ready
    /// to be deduped, persisted, and merged into the transcript.
    HistoryFetched(String, Vec<ChatMessage>),
    /// A `/send`ed file finished importing into the local blob store and
    /// is ready to broadcast -- see `Net::send_file`. Carries the fully
    /// composed message so the caller can broadcast, persist, and display
    /// it exactly like an incoming one.
    FileReady(String, ChatMessage),
    /// A `/send`ed file failed to import -- see `Net::send_file`.
    FileSendFailed { path: String, error: String },
    /// A `/save`d file finished downloading and was written to disk --
    /// see `Net::save_file`.
    FileSaved { filename: String, path: PathBuf },
    /// A `/save`d file failed to download or write to disk -- see
    /// `Net::save_file`.
    FileSaveFailed { filename: String, error: String },
}

/// A channel's gossip topic is derived from its private `RoomSecret`, not
/// its human-readable name -- see `RoomSecret`'s doc comment for why: this
/// is what makes a room unguessable from its name alone, and lets two
/// different rooms share a display name without ever colliding. The
/// `"leyline-room:"` prefix just keeps this hash in its own namespace, in
/// case the secret is ever reused to derive something else later (e.g. a
/// message encryption key).
fn topic_for_secret(secret: &RoomSecret) -> TopicId {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"leyline-room:");
    hasher.update(secret.as_bytes());
    hasher.finalize().into()
}

/// Interprets a `/join` argument as either a bare channel name or an
/// invite ticket.
///
/// If `arg` doesn't even have the ticket prefix, it's treated as a plain
/// channel name with no room secret yet -- see `Net::join`, which
/// generates a fresh one for a name it's never seen before. If it *does*
/// have the prefix but still fails to decode -- e.g. because it was
/// truncated when copied out of a narrow terminal -- that's reported as an
/// error instead of silently joining a bogus channel named after the
/// mangled string.
fn parse_join_arg(arg: &str) -> Result<(String, Option<(RoomSecret, EndpointAddr)>), String> {
    match ChannelTicket::decode_string(arg) {
        Ok(ticket) => Ok((ticket.name, Some((ticket.secret, ticket.addr)))),
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

/// A joined channel's live gossip sender plus the room secret that put us
/// there -- see `RoomSecret` and `topic_for_secret`. Bundled together
/// (rather than two parallel maps keyed by name) so the two can never
/// drift out of sync with each other.
///
/// Also holds the background tasks feeding this channel's `GossipReceiver`
/// (`forward_events`) and retrying its bootstrap join (`retry_join`, if
/// spawned). `iroh-gossip` only actually leaves a topic once *both* its
/// `GossipSender` and `GossipReceiver` halves are dropped -- since
/// `forward_events` holds the receiver in an unbounded loop, `Net::leave`
/// has to abort these tasks explicitly to drop it.
struct JoinedChannel {
    secret: RoomSecret,
    sender: GossipSender,
    tasks: Vec<JoinHandle<()>>,
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
    /// One entry per joined channel, keyed by its local display name.
    /// Shared with spawned join tasks, which insert their entry here once
    /// subscribed -- see `Net::join`.
    channels: Arc<Mutex<HashMap<String, JoinedChannel>>>,
    events_tx: mpsc::Sender<NetEvent>,
    /// Backs history backfill (see backfill.rs): serves our blobs to peers
    /// that fetch from us, and lets us fetch from a peer that announced a
    /// root hash we don't have yet.
    backfill: BackfillStore,
    /// Our current broadcast nickname, if `/nick` has been run this session
    /// -- re-sent to each channel's newly-up neighbors the same way a
    /// `HistoryAnnounce` is (see `announce_nickname`), so a peer who
    /// connects after we set it still learns it. Never persisted, unlike
    /// `contacts.rs`'s petnames -- see features.md's "Broadcast nicknames".
    nickname: Mutex<Option<String>>,
    /// Our own endpoint id, as raw bytes so callers don't need to depend on
    /// iroh types.
    pub our_id: [u8; 32],
}

impl Net {
    /// Binds an endpoint using `secret_key`, then joins the default
    /// "general" channel plus every channel in `known_channels` (the
    /// previous session's `ChannelRegistry`, so nothing joined last time is
    /// forgotten -- see `crate::channel_registry`), seeding bootstrap peers
    /// from each channel's stored addresses and reusing each channel's
    /// already-known `RoomSecret` rather than generating a new one. If
    /// `join_ticket` names a channel not already covered, that channel is
    /// joined too, using the ticket's secret and address. A channel with no
    /// prior secret at all -- i.e. "general" on a genuinely first run, with
    /// no `--join` ticket for it either -- gets a freshly generated one, so
    /// even your very first, un-shared "general" is its own private room
    /// (see `RoomSecret`'s doc comment). Spawns a gossip task per joined
    /// channel forwarding events to `events_tx`. Returns the names of the
    /// channels joined, in join order, plus the name of the channel that
    /// should start active (the `--join` ticket's channel, if one was given
    /// and usable, else "general").
    pub async fn start(
        secret_key: SecretKey,
        join_ticket: Option<String>,
        known_channels: Vec<(String, RoomSecret, Vec<EndpointAddr>)>,
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

        // Wait (briefly) for a home relay so the address used for our own
        // tickets below has a usable relay hint, not just a bare id.
        // `online()` has no timeout of its own -- iroh's own docs warn it
        // will await indefinitely if no relay is reachable (no internet, a
        // captive portal, a restrictive firewall, ...) -- so bound it
        // ourselves rather than hanging the whole app, including the TUI,
        // on network access we may never get. iroh keeps retrying in the
        // background regardless of this timeout; local data, joined
        // channels, and the TUI should all still come up without a relay,
        // so a peer with a directly reachable address (e.g. on the same
        // LAN) can still connect even then.
        if timeout(Duration::from_secs(NET_REPORT_TIMEOUT), endpoint.online())
            .await
            .is_err()
        {
            warn!("no relay reachable within {NET_REPORT_TIMEOUT}s yet; continuing without one");
        }

        let gossip = Gossip::builder().spawn(endpoint.clone());
        let router = Router::builder(endpoint)
            .accept(GOSSIP_ALPN, gossip.clone())
            .accept(iroh_blobs::ALPN, backfill.protocol_handler())
            .spawn();

        let net = Self {
            router,
            gossip,
            address_lookup,
            channels: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            backfill,
            nickname: Mutex::new(None),
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

        // The default channel's room secret: reuse what we already know it
        // as (from a previous session), else the `--join` ticket's secret
        // if it names "general", else generate a brand-new one -- see this
        // method's doc comment on why a fresh, un-shared "general" is
        // still its own private room rather than a well-known public one.
        let general_known = known_channels
            .iter()
            .find(|(name, _, _)| name.as_str() == DEFAULT_CHANNEL);
        let general_secret = general_known
            .map(|(_, secret, _)| *secret)
            .or_else(|| {
                ticket
                    .as_ref()
                    .filter(|t| t.name == DEFAULT_CHANNEL)
                    .map(|t| t.secret)
            })
            .unwrap_or_else(RoomSecret::generate);
        let mut general_bootstrap = general_known
            .map(|(_, _, addrs)| addrs.clone())
            .unwrap_or_default();
        if let Some(t) = ticket.as_ref().filter(|t| t.name == DEFAULT_CHANNEL) {
            general_bootstrap.push(t.addr.clone());
        }
        net.subscribe_and_register(DEFAULT_CHANNEL, general_secret, general_bootstrap)
            .await
            .context("failed to join default channel")?;
        joined.push(DEFAULT_CHANNEL.to_string());

        for (name, secret, addrs) in &known_channels {
            if name.as_str() == DEFAULT_CHANNEL {
                continue;
            }
            let mut bootstrap = addrs.clone();
            if let Some(t) = ticket.as_ref().filter(|t| t.name.as_str() == name.as_str()) {
                bootstrap.push(t.addr.clone());
            }
            net.subscribe_and_register(name, *secret, bootstrap)
                .await
                .with_context(|| format!("failed to rejoin channel {name:?}"))?;
            joined.push(name.clone());
        }

        let active = match &ticket {
            Some(t) if !joined.contains(&t.name) => {
                net.subscribe_and_register(&t.name, t.secret, vec![t.addr.clone()])
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

    /// Subscribes to `secret`'s topic, registers `name` to the resulting
    /// sender (plus `secret`, so `ticket_for`/`secret_for` can look it back
    /// up later), and spawns its forwarding (and, if `bootstrap` is
    /// non-empty, retry-join) tasks. Shared by startup (`Net::start`) and
    /// runtime (`Net::join`) joins so the two paths can't drift apart.
    /// `bootstrap` may combine addresses from more than one source (e.g. a
    /// `--join` ticket and previously-learned peers from
    /// `crate::channel_registry`) -- duplicates are harmless, since
    /// iroh-gossip dedupes its bootstrap set internally.
    async fn subscribe_and_register(
        &self,
        name: &str,
        secret: RoomSecret,
        bootstrap: Vec<EndpointAddr>,
    ) -> anyhow::Result<()> {
        for addr in &bootstrap {
            self.address_lookup.add_endpoint_info(addr.clone());
        }
        let bootstrap_ids: Vec<EndpointId> = bootstrap.iter().map(|addr| addr.id).collect();

        let topic = topic_for_secret(&secret);
        let (sender, receiver) = self
            .gossip
            .subscribe(topic, bootstrap_ids.clone())
            .await
            .context("failed to subscribe to gossip topic")?
            .split();
        debug!(channel = %name, bootstrap = bootstrap_ids.len(), "joined channel");

        let mut tasks = Vec::new();
        if !bootstrap_ids.is_empty() {
            tasks.push(tokio::spawn(retry_join(sender.clone(), bootstrap_ids)));
        }
        tasks.push(tokio::spawn(forward_events(
            name.to_string(),
            receiver,
            self.events_tx.clone(),
            self.router.endpoint().clone(),
        )));

        self.channels
            .lock()
            .expect("channels lock poisoned")
            .insert(
                name.to_string(),
                JoinedChannel {
                    secret,
                    sender,
                    tasks,
                },
            );

        Ok(())
    }

    /// The room secret backing `name`, if it's currently joined -- so
    /// callers (e.g. `main.rs`, to persist it to
    /// `crate::channel_registry`) can look up what `Net` resolved or
    /// generated for a channel once it's joined. `None` if `name` isn't
    /// joined.
    pub fn secret_for(&self, name: &str) -> Option<RoomSecret> {
        self.channels
            .lock()
            .expect("channels lock poisoned")
            .get(name)
            .map(|c| c.secret)
    }

    /// Joins (or creates) a channel named by a bare name or an invite
    /// ticket string.
    ///
    /// A bare name that isn't already joined creates a brand-new room with
    /// a freshly generated `RoomSecret` -- private until you `/invite`
    /// someone into it. A ticket's secret is used as-is. If `name` is
    /// *already* joined, its existing secret always wins: a bare name is a
    /// no-op re-subscribe, while a ticket whose secret doesn't match is
    /// rejected with an explanatory message instead of silently switching
    /// rooms out from under an existing tab -- names are just local
    /// labels, so two unrelated rooms can otherwise happen to want the
    /// same one.
    ///
    /// Returns `Some(message)` immediately if `arg` looks like a ticket but
    /// fails to decode (see `parse_join_arg`), if it names a different
    /// room than one we already have under that name (see above), or if it
    /// decodes but names our own identity (see `is_self`) -- in the
    /// identity case the join below still proceeds (using no bootstrap,
    /// since a self-referential one is useless) so the channel is
    /// created/switched to regardless. Otherwise this is fire-and-forget:
    /// spawns its own task and reports the outcome back as a
    /// `NetEvent::Joined`/`JoinFailed`, so the caller's event loop never
    /// awaits network I/O directly.
    pub fn join(&self, arg: String) -> Option<String> {
        let (name, ticket) = match parse_join_arg(&arg) {
            Ok(parsed) => parsed,
            Err(error) => return Some(error),
        };
        if name.is_empty() {
            return None;
        }

        let existing_secret = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .get(&name)
            .map(|c| c.secret);
        let ticket_secret = ticket.as_ref().map(|(secret, _)| *secret);
        if let (Some(existing), Some(from_ticket)) = (existing_secret, ticket_secret)
            && existing != from_ticket
        {
            return Some(format!(
                "that ticket is for a different channel also named #{name} -- pick a new local name to join it"
            ));
        }
        if existing_secret.is_some() {
            let _ = self.events_tx.try_send(NetEvent::Joined(name));
            return None;
        }
        let secret = ticket_secret.unwrap_or_else(RoomSecret::generate);
        let bootstrap = ticket.map(|(_, addr)| addr);

        let self_ticket = bootstrap
            .as_ref()
            .is_some_and(|addr| is_self(addr, self.our_id));
        let bootstrap = if self_ticket { None } else { bootstrap };
        let message = self_ticket.then(|| {
            format!(
                "that invite ticket points at your own identity, so it can't connect you to anyone -- joining #{name} anyway"
            )
        });

        let topic = topic_for_secret(&secret);
        let gossip = self.gossip.clone();
        let address_lookup = self.address_lookup.clone();
        let channels = Arc::clone(&self.channels);
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
                    let mut tasks = Vec::new();
                    if !bootstrap_ids.is_empty() {
                        tasks.push(tokio::spawn(retry_join(sender.clone(), bootstrap_ids)));
                    }
                    tasks.push(tokio::spawn(forward_events(
                        name.clone(),
                        receiver,
                        events_tx.clone(),
                        endpoint,
                    )));
                    channels.lock().expect("channels lock poisoned").insert(
                        name.clone(),
                        JoinedChannel {
                            secret,
                            sender,
                            tasks,
                        },
                    );
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

    /// Leaves a previously joined channel: drops its gossip subscription
    /// and stops its background tasks. `iroh-gossip` only actually leaves
    /// a topic once both the `GossipSender` and `GossipReceiver` halves
    /// are dropped (see `JoinedChannel`'s doc comment) -- removing the map
    /// entry drops our `GossipSender`, and aborting `tasks` drops the
    /// `GossipReceiver` held by `forward_events`, which would otherwise
    /// run forever.
    ///
    /// A no-op (logged) if `channel` isn't currently joined -- the caller
    /// (`app::AppState::run_leave`) only ever names a channel it already
    /// has a tab for, which is only ever true once it's actually joined.
    pub fn leave(&self, channel: &str) {
        let removed = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .remove(channel);
        match removed {
            Some(joined) => {
                for task in joined.tasks {
                    task.abort();
                }
                debug!(%channel, "left channel");
            }
            None => warn!(%channel, "tried to leave a channel that wasn't joined"),
        }
    }

    /// Encodes and broadcasts a chat message to `channel`. Fire-and-forget:
    /// spawns its own task so the caller's event loop never awaits network
    /// I/O directly. Drops the message if `channel` isn't joined -- the UI
    /// shouldn't be able to compose a message for a channel it doesn't know
    /// about, so this would indicate a bug elsewhere.
    pub fn send(&self, channel: &str, message: ChatMessage) {
        let Some(sender) = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .get(channel)
            .map(|c| c.sender.clone())
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

    /// Announces our current history root hash for `channel` to the whole
    /// channel -- see backfill.rs. Called whenever a channel gains a
    /// neighbor, in either direction, so both a fresh join and a reconnect
    /// give every side a chance to notice they're missing something, and
    /// again periodically regardless of neighbor churn (see main.rs's
    /// history-announce heartbeat), so peers who never become each other's
    /// direct gossip neighbor still eventually hear about each other's
    /// history. Uses a full-mesh `broadcast`, the same as `send` and
    /// `set_nickname` -- unlike `announce_nickname`, this deliberately isn't
    /// neighbor-scoped: iroh-gossip's HyParView layer only keeps a handful
    /// of peers (its "active view") as direct neighbors at once, so once a
    /// channel outgrows that, most members would otherwise never receive
    /// each other's announces and could only ever backfill from whichever
    /// peer they happened to bootstrap through. Fire-and-forget, like `send`.
    pub fn announce(&self, channel: &str, root: Hash) {
        let Some(sender) = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .get(channel)
            .map(|c| c.sender.clone())
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
            if let Err(err) = sender.broadcast(bytes.into()).await {
                warn!("failed to broadcast history announce: {err}");
            }
        });
    }

    /// Sets our broadcast nickname and immediately floods it to every
    /// currently-joined channel via a full `broadcast` (like `send` -- not
    /// neighbor-scoped, since a `/nick` should propagate through the whole
    /// mesh like a chat message would). Fire-and-forget, like `send`.
    pub fn set_nickname(&self, nickname: String) {
        *self.nickname.lock().expect("nickname lock poisoned") = Some(nickname.clone());
        let senders: Vec<GossipSender> = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .values()
            .map(|c| c.sender.clone())
            .collect();
        let payload = GossipPayload::Identity(IdentityAnnounce {
            sender: self.our_id,
            nickname,
        });
        tokio::spawn(async move {
            let bytes = match postcard::to_stdvec(&payload) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("failed to encode identity announce: {err}");
                    return;
                }
            };
            for sender in senders {
                if let Err(err) = sender.broadcast(bytes.clone().into()).await {
                    warn!("failed to broadcast identity announce: {err}");
                }
            }
        });
    }

    /// Re-announces our current nickname (if `/nick` has been run this
    /// session) to `channel`'s direct gossip neighbors -- called whenever a
    /// channel gains one, the same way `announce` re-sends the history
    /// root, so a peer who connects after we set our nickname still learns
    /// it. A no-op if no nickname has been set yet. Fire-and-forget, like
    /// `announce`.
    pub fn announce_nickname(&self, channel: &str) {
        let Some(nickname) = self
            .nickname
            .lock()
            .expect("nickname lock poisoned")
            .clone()
        else {
            return;
        };
        let Some(sender) = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .get(channel)
            .map(|c| c.sender.clone())
        else {
            return;
        };
        let payload = GossipPayload::Identity(IdentityAnnounce {
            sender: self.our_id,
            nickname,
        });
        tokio::spawn(async move {
            let bytes = match postcard::to_stdvec(&payload) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("failed to encode identity announce: {err}");
                    return;
                }
            };
            if let Err(err) = sender.broadcast_neighbors(bytes.into()).await {
                warn!("failed to broadcast identity announce: {err}");
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

    /// Imports `path` into the shared blob store and prepares it to be
    /// broadcast to `channel` as a `ChatMessage` with an attachment -- the
    /// `/send` counterpart to `send`. Validates synchronously (existence,
    /// that it's a file rather than a directory, a simple size cap, and
    /// that a filename can be derived) so an obviously bad path is
    /// reported immediately instead of after spawning work; the import
    /// itself happens in a spawned task, reporting the outcome back as
    /// `NetEvent::FileReady`/`FileSendFailed` since hashing a large file
    /// can take a moment and must not block the caller's event loop.
    /// Deliberately does *not* broadcast itself -- the caller (main.rs)
    /// does that via `send` once it receives `FileReady`, the same place
    /// every other locally-authored message is broadcast from.
    pub fn send_file(&self, channel: String, path: String) -> Option<String> {
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) => return Some(format!("can't send {path}: {err}")),
        };
        if !metadata.is_file() {
            return Some(format!("can't send {path}: not a file"));
        }
        if metadata.len() > MAX_FILE_SIZE {
            return Some(format!(
                "can't send {path}: too large ({}, max {})",
                crate::files::human_size(metadata.len()),
                crate::files::human_size(MAX_FILE_SIZE)
            ));
        }
        let Some(filename) = Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            return Some(format!("can't send {path}: can't determine a filename"));
        };

        let backfill = self.backfill.clone();
        let events_tx = self.events_tx.clone();
        let our_id = self.our_id;
        let size = metadata.len();
        tokio::spawn(async move {
            let hash = match backfill.add_file(Path::new(&path)).await {
                Ok(hash) => hash,
                Err(err) => {
                    let _ = events_tx
                        .send(NetEvent::FileSendFailed {
                            path,
                            error: err.to_string(),
                        })
                        .await;
                    return;
                }
            };
            let message = ChatMessage {
                v: 2,
                id: rand::random(),
                sender: our_id,
                ts_unix_ms: now_unix_ms(),
                text: String::new(),
                attachment: Some(FileAttachment {
                    filename,
                    size,
                    hash,
                }),
            };
            let _ = events_tx.send(NetEvent::FileReady(channel, message)).await;
        });

        None
    }

    /// Downloads a file previously shared in some joined channel and writes
    /// it to `destination` -- the `/save` counterpart to `send_file`.
    /// `destination` is already fully resolved by the caller (see
    /// `files::resolve_destination`), so this is purely a network/blob-store
    /// concern. Spawned like `send_file`, reporting the outcome back as
    /// `NetEvent::FileSaved`/`FileSaveFailed` since the download can take a
    /// while and must not block the caller's event loop. Fire-and-forget,
    /// like `send` and `announce`.
    pub fn save_file(&self, hash: Hash, filename: String, sender: [u8; 32], destination: PathBuf) {
        let backfill = self.backfill.clone();
        let endpoint = self.router.endpoint().clone();
        let events_tx = self.events_tx.clone();
        tokio::spawn(async move {
            match backfill
                .fetch_file(&endpoint, hash, sender, &destination)
                .await
            {
                Ok(()) => {
                    let _ = events_tx
                        .send(NetEvent::FileSaved {
                            filename,
                            path: destination,
                        })
                        .await;
                }
                Err(err) => {
                    let _ = events_tx
                        .send(NetEvent::FileSaveFailed {
                            filename,
                            error: err.to_string(),
                        })
                        .await;
                }
            }
        });
    }

    /// Builds this instance's invite ticket string for `channel`: our
    /// current address, the channel's room secret, and its display name,
    /// ready to be pasted into another instance's `--join` flag or `/join`
    /// command. Synchronous -- `Endpoint::addr` doesn't need to await
    /// anything.
    ///
    /// # Panics
    /// Panics if `channel` isn't currently joined -- every call site
    /// (startup's own just-joined list, and `/invite`'s active channel)
    /// only ever names a channel we're already in.
    pub fn ticket_for(&self, channel: &str) -> String {
        let secret = self
            .secret_for(channel)
            .expect("ticket_for called for an unjoined channel");
        ChannelTicket {
            name: channel.to_string(),
            secret,
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
                    Ok(GossipPayload::Identity(identity)) => {
                        NetEvent::Identity(channel.clone(), identity)
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
        let (name, ticket) = parse_join_arg("project-x").unwrap();
        assert_eq!(name, "project-x");
        assert!(ticket.is_none());
    }

    #[test]
    fn parse_join_arg_decodes_a_valid_ticket() {
        let addr = sample_addr();
        let secret = RoomSecret::generate();
        let encoded = ChannelTicket {
            name: "general".to_string(),
            secret,
            addr: addr.clone(),
        }
        .encode_string();

        let (name, ticket) = parse_join_arg(&encoded).unwrap();
        assert_eq!(name, "general");
        assert_eq!(ticket, Some((secret, addr)));
    }

    #[test]
    fn parse_join_arg_reports_a_truncated_ticket_as_an_error_not_a_name() {
        let ticket = ChannelTicket {
            name: "general".to_string(),
            secret: RoomSecret::generate(),
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

    #[test]
    fn topic_for_secret_differs_for_different_secrets_even_with_the_same_name() {
        // The whole point of deriving the topic from the secret rather
        // than the name: two rooms both called "general" must not land on
        // the same swarm.
        let a = topic_for_secret(&RoomSecret::generate());
        let b = topic_for_secret(&RoomSecret::generate());
        assert_ne!(a, b);
    }

    #[test]
    fn topic_for_secret_is_deterministic_for_the_same_secret() {
        let secret = RoomSecret::generate();
        assert_eq!(topic_for_secret(&secret), topic_for_secret(&secret));
    }
}
