//! Point-to-point 1:1 messaging over a direct QUIC stream to a known
//! `EndpointId` -- no gossip topic, `RoomSecret`, or invite ticket, unlike
//! every other channel kind in this app (see concept.md's "Identity &
//! channels"). See features/LEY-12's "True direct 1:1 messaging over a
//! point-to-point stream".
//!
//! Mirrors `pairing.rs`/`backfill.rs`'s relationship to `net.rs`: this
//! module owns its own ALPN-registered protocol handler and in-memory
//! state, and `net.rs` just registers it on the shared `Router` and
//! forwards a few thin wrapper calls. Deliberately has no dependency on
//! `app.rs` -- like `net.rs` itself, this is network-layer code that only
//! ever talks to the rest of the app via `NetEvent`s.
//!
//! A point-to-point conversation is represented on the UI side as an
//! ordinary channel tab (see `app::Channel`), named via `channel_name`
//! below and persisted through the exact same `storage::MessageStore` any
//! other channel uses -- there is no new transcript, search, or thread
//! machinery here, only the connection lifecycle: dialing, retrying on
//! disconnect, accepting an inbound connection from a peer who reaches us
//! first, and framing/decoding messages on top of raw QUIC streams.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointId};
use iroh_docs::Capability;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::message::ChatMessage;
use crate::net::NetEvent;

/// ALPN for point-to-point messaging -- registered on the same `Router` as
/// gossip, blobs, and pairing (see `net.rs`), but never carries gossip
/// traffic: each connection is a direct, two-party conversation.
pub const DIRECT_ALPN: &[u8] = b"leyline/direct/1";

/// Generous upper bound on one message's encoded size -- a guardrail
/// against a broken or malicious peer streaming unbounded data on a uni
/// stream, not a real limit on an ordinary chat message (mirrors
/// `pairing.rs`'s `MAX_PAYLOAD_BYTES`).
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const CONTROL_PREFIX: &[u8] = b"leyline/direct-control/1:";

#[derive(Serialize, Deserialize)]
enum Control {
    Docs(Capability),
}

/// How many outstanding commands (sends, or inbound connection hand-offs)
/// a peer's actor will buffer before `Direct::send` starts dropping
/// messages -- generous for a single 1:1 conversation's traffic.
const COMMAND_QUEUE_CAPACITY: usize = 16;

/// Initial delay before the first retry after a failed or dropped
/// connection, doubling on each consecutive failure up to
/// `RECONNECT_MAX_DELAY` -- see `run_actor`. Reset to this floor after any
/// successful connect, so a peer that's merely blipped reconnects quickly
/// rather than inheriting a long wait from an earlier, longer outage.
const RECONNECT_MIN_DELAY: Duration = Duration::from_millis(500);

/// Upper bound on the retry backoff -- long enough to stop hammering a
/// peer that's genuinely offline for a while, short enough that a 1:1
/// conversation still feels like it reconnects "eventually" without
/// needing a manual nudge.
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);

/// The local display name for `peer`'s point-to-point channel -- the
/// single source of truth both the outbound (`Direct::open`/`/dm`) and
/// inbound (`Direct::accept`) paths compute it from, so a conversation
/// always converges on the same tab and the same `storage::MessageStore`
/// history file regardless of who connects first. Deliberately not
/// cosmetic (no pet name) and has no collision-suffix fallback, unlike
/// `app::AppState::unique_dm_channel_name` for gossip DMs -- it has to stay
/// stable and recomputable from the peer id alone, since nothing else
/// durable records the association (see `direct_registry.rs`).
pub fn channel_name(peer: &[u8; 32]) -> String {
    format!("p2p-{}", hex_prefix(peer))
}

/// Shortened hex id for `channel_name` and log lines -- a standalone copy
/// of `app::hex_prefix` rather than a shared import: this module (owned by
/// `net::Net`) is network-layer code and, like `net.rs` itself, has no
/// dependency on the UI layer (`app.rs`).
fn hex_prefix(bytes: &[u8; 32]) -> String {
    bytes[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// What a peer's actor (`run_actor`) reacts to: a message to send once
/// connected, or a freshly accepted inbound connection to adopt.
enum Command {
    Send(ChatMessage),
    ShareDocs(Capability),
    Inbound(Connection),
}

/// A running peer actor: the handle used to queue commands for it, and the
/// task itself, so `Direct::close` can abort it outright.
#[derive(Debug)]
struct PeerActor {
    commands: mpsc::Sender<Command>,
    handle: JoinHandle<()>,
}

/// Owns one background actor per peer we're (or might be) having a
/// point-to-point conversation with, and the `ProtocolHandler` that routes
/// an inbound `DIRECT_ALPN` connection to the right one -- see `net.rs`,
/// which constructs this once at startup, registers it on the shared
/// `Router`, and exposes `open`/`send`/`close` as thin wrapper methods on
/// `Net`.
#[derive(Debug)]
pub struct Direct {
    endpoint: Endpoint,
    events_tx: mpsc::Sender<NetEvent>,
    actors: Mutex<HashMap<[u8; 32], PeerActor>>,
    docs: Arc<Mutex<HashMap<[u8; 32], Capability>>>,
}

impl Direct {
    pub fn new(endpoint: Endpoint, events_tx: mpsc::Sender<NetEvent>) -> Self {
        Self {
            endpoint,
            events_tx,
            actors: Mutex::new(HashMap::new()),
            docs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Ensures a background actor is running for `peer`, spawning one if
    /// this is the first time it's been mentioned this session (by us
    /// dialing out, or by them reaching us first). Returns a handle to
    /// queue commands on it, plus whether it was newly created -- only
    /// `accept` below needs that second part, to decide whether this is a
    /// first contact worth a `NetEvent::DirectJoined`.
    fn ensure_actor(&self, peer: [u8; 32]) -> (mpsc::Sender<Command>, bool) {
        let mut actors = self.actors.lock().expect("direct actors lock poisoned");
        if let Some(actor) = actors.get(&peer) {
            return (actor.commands.clone(), false);
        }
        let (commands_tx, commands_rx) = mpsc::channel(COMMAND_QUEUE_CAPACITY);
        let handle = tokio::spawn(run_actor(
            peer,
            self.endpoint.clone(),
            commands_rx,
            self.events_tx.clone(),
            Arc::clone(&self.docs),
        ));
        actors.insert(
            peer,
            PeerActor {
                commands: commands_tx.clone(),
                handle,
            },
        );
        (commands_tx, true)
    }

    /// Ensures `peer`'s connect-or-reconnect loop is running and returns
    /// its channel name (`channel_name`) -- the point-to-point counterpart
    /// to `net::Net::join`'s fire-and-forget subscribe, but fully
    /// synchronous: unlike a gossip subscribe, there's no network round
    /// trip to await before a tab can be considered "open". Idempotent --
    /// calling this again for an already-running peer is a no-op beyond
    /// returning the same name.
    pub fn open(&self, peer: [u8; 32]) -> String {
        self.ensure_actor(peer);
        channel_name(&peer)
    }

    /// Queues `message` to be sent to `peer` once connected, starting
    /// their actor first if this is the first contact with them. A
    /// message sent while disconnected is held (see `run_actor`) and
    /// flushed as soon as a connection comes up, rather than dropped.
    /// Fire-and-forget, like `net::Net::send`: synchronous, and a full
    /// queue (an unresponsive or wedged actor) is logged and the message
    /// dropped rather than blocking the caller's event loop.
    pub fn send(&self, peer: [u8; 32], message: ChatMessage) {
        let (commands, _) = self.ensure_actor(peer);
        if commands.try_send(Command::Send(message)).is_err() {
            warn!(peer = %hex_prefix(&peer), "dropping direct message: actor unavailable");
        }
    }

    pub fn set_docs(&self, peer: [u8; 32], docs: Capability) {
        self.docs
            .lock()
            .expect("direct docs lock poisoned")
            .insert(peer, docs.clone());
        let (commands, _) = self.ensure_actor(peer);
        if commands.try_send(Command::ShareDocs(docs)).is_err() {
            warn!(peer = %hex_prefix(&peer), "direct docs capability will wait for reconnect");
        }
    }

    /// Stops `peer`'s actor outright -- e.g. `/leave`ing a point-to-point
    /// channel. Aborts the task directly (mirroring `net::Net::leave`'s
    /// `JoinHandle::abort()`) rather than a graceful shutdown handshake,
    /// since there's nothing left to flush once the caller has decided to
    /// stop talking to this peer. A no-op if `peer` has no running actor.
    pub fn close(&self, peer: [u8; 32]) {
        if let Some(actor) = self
            .actors
            .lock()
            .expect("direct actors lock poisoned")
            .remove(&peer)
        {
            actor.handle.abort();
        }
        self.docs
            .lock()
            .expect("direct docs lock poisoned")
            .remove(&peer);
    }
}

async fn send_control(connection: &Connection, control: Control) {
    let bytes = match postcard::to_stdvec(&control) {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!("failed to encode direct control frame: {err}");
            return;
        }
    };
    let mut send_stream = match connection.open_uni().await {
        Ok(stream) => stream,
        Err(err) => {
            warn!("failed to open direct control stream: {err}");
            return;
        }
    };
    if let Err(err) = send_stream.write_all(CONTROL_PREFIX).await {
        warn!("failed to send direct control prefix: {err}");
        return;
    }
    if let Err(err) = send_stream.write_all(&bytes).await {
        warn!("failed to send direct control frame: {err}");
        return;
    }
    if let Err(err) = send_stream.finish() {
        warn!("failed to finish direct control stream: {err}");
    }
}

impl ProtocolHandler for Direct {
    /// Accepts an inbound point-to-point connection: hands it to `peer`'s
    /// actor, starting one first if they've never reached us this session.
    /// Only that first-contact case reports `NetEvent::DirectJoined`, so
    /// `app.rs` can create a background tab for it (mirroring
    /// `net::NetEvent::ChannelSyncJoined`'s "don't steal focus" semantics,
    /// since the local user never asked for this conversation to appear).
    /// A routine reconnect from an already-known peer only ever updates
    /// presence (`NetEvent::PeerJoined`, emitted by the actor itself once
    /// it adopts this connection), not this event.
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer = *connection.remote_id().as_bytes();
        let (commands, is_new) = self.ensure_actor(peer);
        if is_new {
            let _ = self
                .events_tx
                .send(NetEvent::DirectJoined {
                    channel: channel_name(&peer),
                    peer,
                })
                .await;
        }
        if commands.send(Command::Inbound(connection)).await.is_err() {
            warn!(peer = %hex_prefix(&peer), "dropping inbound direct connection: actor unavailable");
        }
        Ok(())
    }
}

/// Aborts the wrapped task when dropped -- used for the per-connection
/// `receive_loop` subtask below, so it's never leaked. `run_actor`'s own
/// task can end in several ways (looping around to redial, returning
/// normally, or being force-aborted outright by `Direct::close`), and
/// every one of them needs the receive loop for whatever connection was
/// current at the time to stop too -- a plain `JoinHandle` left in a local
/// variable does not do this on its own (dropping a `JoinHandle` merely
/// detaches the task, it keeps running), but wrapping it here means
/// reassigning or simply letting this value go out of scope -- including
/// via task cancellation, which still runs `Drop` for live locals -- always
/// cleans it up.
struct AbortOnDrop(JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// One peer's connection lifecycle, run as a background task for as long
/// as `Direct::close` hasn't aborted it (which happens on `/leave`, or
/// never, for the life of the process): dial (or accept) a connection,
/// serve it until it closes, then retry with backoff -- AC #2's "recovers
/// from a disconnect via retry/lifecycle handling". Messages queued via
/// `Command::Send` while disconnected are held and flushed once a
/// connection comes up, never dropped just for arriving at a bad moment.
///
/// Holds at most one live connection at a time. If both sides happen to
/// connect to each other around the same time, or an inbound hand-off
/// arrives while a dial is still in flight, whichever connection becomes
/// current most recently simply wins -- no simultaneous-open tie-breaking.
/// This is safe without it: sends only ever go out over the current
/// connection, and `app::Channel`'s existing id-based dedupe already
/// protects the UI from any accidental double-delivery during a handover.
async fn run_actor(
    peer: [u8; 32],
    endpoint: Endpoint,
    mut commands: mpsc::Receiver<Command>,
    events_tx: mpsc::Sender<NetEvent>,
    docs: Arc<Mutex<HashMap<[u8; 32], Capability>>>,
) {
    let channel = channel_name(&peer);
    let Ok(peer_id) = EndpointId::from_bytes(&peer) else {
        warn!(%channel, "invalid direct peer id; not starting a connection");
        return;
    };

    let mut pending: Vec<ChatMessage> = Vec::new();
    let mut backoff = RECONNECT_MIN_DELAY;

    loop {
        let mut connection = loop {
            tokio::select! {
                result = endpoint.connect(peer_id, DIRECT_ALPN) => {
                    match result {
                        Ok(conn) => break conn,
                        Err(err) => {
                            debug!(%channel, "direct connect attempt failed, retrying in {backoff:?}: {err}");
                            tokio::time::sleep(backoff).await;
                            backoff = (backoff * 2).min(RECONNECT_MAX_DELAY);
                        }
                    }
                }
                command = commands.recv() => {
                    match command {
                        Some(Command::Send(message)) => pending.push(message),
                        Some(Command::ShareDocs(_)) => {}
                        Some(Command::Inbound(conn)) => break conn,
                        None => return,
                    }
                }
            }
        };
        backoff = RECONNECT_MIN_DELAY;
        if events_tx
            .send(NetEvent::PeerJoined(channel.clone(), peer))
            .await
            .is_err()
        {
            return;
        }
        let capability = docs
            .lock()
            .expect("direct docs lock poisoned")
            .get(&peer)
            .cloned();
        if let Some(capability) = capability {
            send_control(&connection, Control::Docs(capability)).await;
        }

        for message in pending.drain(..) {
            send_one(&connection, &message).await;
        }

        let mut receiver = AbortOnDrop(tokio::spawn(receive_loop(
            connection.clone(),
            channel.clone(),
            events_tx.clone(),
        )));

        loop {
            tokio::select! {
                _ = connection.closed() => break,
                command = commands.recv() => {
                    match command {
                        Some(Command::Send(message)) => send_one(&connection, &message).await,
                        Some(Command::ShareDocs(capability)) => {
                            send_control(&connection, Control::Docs(capability)).await;
                        }
                        Some(Command::Inbound(new_connection)) => {
                            connection = new_connection;
                            let capability = docs
                                .lock()
                                .expect("direct docs lock poisoned")
                                .get(&peer)
                                .cloned();
                            if let Some(capability) = capability {
                                send_control(&connection, Control::Docs(capability)).await;
                            }
                            receiver = AbortOnDrop(tokio::spawn(receive_loop(
                                connection.clone(),
                                channel.clone(),
                                events_tx.clone(),
                            )));
                        }
                        None => {
                            let _ = events_tx.send(NetEvent::PeerLeft(channel.clone(), peer)).await;
                            return;
                        }
                    }
                }
            }
        }
        drop(receiver);

        if events_tx
            .send(NetEvent::PeerLeft(channel.clone(), peer))
            .await
            .is_err()
        {
            return;
        }
    }
}

/// Encodes and sends `message` over a fresh uni stream -- one stream per
/// message, mirroring `pairing.rs`'s one-exchange-per-stream shape rather
/// than storage.rs's length-prefixed framing, so no manual framing is
/// needed on top of what QUIC streams already give for free. Logged and
/// dropped on failure, like every other fire-and-forget send in `net.rs`
/// -- the caller's `connection.closed()` (see `run_actor`) is what
/// actually notices a dead connection, not this.
async fn send_one(connection: &Connection, message: &ChatMessage) {
    let bytes = match postcard::to_stdvec(message) {
        Ok(bytes) => bytes,
        Err(err) => {
            warn!("failed to encode direct message: {err}");
            return;
        }
    };
    let mut send_stream = match connection.open_uni().await {
        Ok(stream) => stream,
        Err(err) => {
            warn!("failed to open direct message stream: {err}");
            return;
        }
    };
    if let Err(err) = send_stream.write_all(&bytes).await {
        warn!("failed to send direct message: {err}");
        return;
    }
    if let Err(err) = send_stream.finish() {
        warn!("failed to finish direct message stream: {err}");
    }
}

/// Reads uni streams off `connection` for as long as it stays open,
/// decoding each one as a `ChatMessage` (via `ChatMessage::decode`'s
/// version-fallback logic, so an older peer's build is still
/// understood) and forwarding it as `NetEvent::Received`. Returns once the
/// connection closes -- an entirely ordinary, expected event for a 1:1
/// conversation, not logged as a warning (see `run_actor`, which reacts to
/// the same closure via `connection.closed()` to emit `PeerLeft`).
async fn receive_loop(connection: Connection, channel: String, events_tx: mpsc::Sender<NetEvent>) {
    let peer = *connection.remote_id().as_bytes();
    loop {
        let mut recv_stream = match connection.accept_uni().await {
            Ok(stream) => stream,
            Err(err) => {
                debug!(%channel, "direct connection closed: {err}");
                return;
            }
        };
        match recv_stream.read_to_end(MAX_MESSAGE_BYTES).await {
            Ok(bytes) if bytes.starts_with(CONTROL_PREFIX) => {
                match postcard::from_bytes::<Control>(&bytes[CONTROL_PREFIX.len()..]) {
                    Ok(Control::Docs(docs)) => {
                        if events_tx
                            .send(NetEvent::DirectDocs { peer, docs })
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    Err(err) => warn!(%channel, "dropping malformed direct control frame: {err}"),
                }
            }
            Ok(bytes) => match ChatMessage::decode(&bytes) {
                Ok(message) => {
                    if events_tx
                        .send(NetEvent::Received(channel.clone(), message))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Err(err) => warn!(%channel, "dropping malformed direct message: {err}"),
            },
            Err(err) => warn!(%channel, "failed to read direct message stream: {err}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use iroh::SecretKey;
    use iroh::address_lookup::memory::MemoryLookup;
    use iroh::endpoint::presets;
    use iroh::protocol::Router;

    use super::*;

    #[test]
    fn channel_name_is_deterministic_for_the_same_peer() {
        let peer = [7; 32];
        assert_eq!(channel_name(&peer), channel_name(&peer));
    }

    #[test]
    fn channel_name_differs_for_different_peers() {
        assert_ne!(channel_name(&[1; 32]), channel_name(&[2; 32]));
    }

    #[test]
    fn channel_name_is_prefixed_for_easy_recognition_in_the_tab_bar() {
        assert!(channel_name(&[1; 32]).starts_with("p2p-"));
    }

    fn sample_message(text: &str) -> ChatMessage {
        ChatMessage {
            v: 3,
            id: rand::random(),
            sender: [9; 32],
            ts_unix_ms: 0,
            text: text.to_string(),
            attachment: None,
            reply_to: None,
        }
    }

    /// Binds a fresh loopback-only endpoint (optionally on a caller-chosen
    /// identity, so a peer can "come back" as the same `EndpointId` after
    /// being torn down -- see the reconnect test below) with its own
    /// `Direct` registered on a `Router`, mirroring `backfill.rs`'s
    /// `fetch_file` test: `presets::Minimal` plus a `MemoryLookup` the test
    /// seeds by hand, so dialing by bare `EndpointId` works without
    /// depending on public relay/discovery infrastructure.
    async fn spawn_direct(
        key: Option<SecretKey>,
    ) -> (
        Endpoint,
        Router,
        Arc<Direct>,
        mpsc::Receiver<NetEvent>,
        MemoryLookup,
    ) {
        let address_lookup = MemoryLookup::new();
        let mut builder =
            Endpoint::builder(presets::Minimal).address_lookup(address_lookup.clone());
        if let Some(key) = key {
            builder = builder.secret_key(key);
        }
        let endpoint = builder.bind().await.unwrap();
        let (events_tx, events_rx) = mpsc::channel(16);
        let direct = Arc::new(Direct::new(endpoint.clone(), events_tx));
        let router = Router::builder(endpoint.clone())
            .accept(DIRECT_ALPN, direct.clone())
            .spawn();
        (endpoint, router, direct, events_rx, address_lookup)
    }

    async fn recv_matching<F: Fn(&NetEvent) -> bool>(
        rx: &mut mpsc::Receiver<NetEvent>,
        matches: F,
    ) -> NetEvent {
        loop {
            let event = tokio::time::timeout(Duration::from_secs(10), rx.recv())
                .await
                .expect("timed out waiting for an expected NetEvent")
                .expect("direct events channel closed unexpectedly");
            if matches(&event) {
                return event;
            }
        }
    }

    #[tokio::test]
    async fn sends_and_receives_a_message_over_a_fresh_connection() {
        let (endpoint_a, router_a, direct_a, _events_a, lookup_a) = spawn_direct(None).await;
        let (endpoint_b, router_b, _direct_b, mut events_b, lookup_b) = spawn_direct(None).await;
        lookup_a.add_endpoint_info(endpoint_b.addr());
        lookup_b.add_endpoint_info(endpoint_a.addr());

        let a_id = *endpoint_a.id().as_bytes();
        let b_id = *endpoint_b.id().as_bytes();
        let message = sample_message("hello from a");

        direct_a.send(b_id, message.clone());

        // B never called `open`/`send` -- this is first contact entirely
        // from A's side, so B's `Direct::accept` must spawn the actor
        // itself and report it.
        let joined = recv_matching(&mut events_b, |e| {
            matches!(e, NetEvent::DirectJoined { .. })
        })
        .await;
        match joined {
            NetEvent::DirectJoined { channel, peer } => {
                assert_eq!(channel, channel_name(&a_id));
                assert_eq!(peer, a_id);
            }
            _ => unreachable!(),
        }

        let received = recv_matching(&mut events_b, |e| matches!(e, NetEvent::Received(..))).await;
        match received {
            NetEvent::Received(channel, got) => {
                assert_eq!(channel, channel_name(&a_id));
                assert_eq!(got, message);
            }
            _ => unreachable!(),
        }

        let _ = (router_a, router_b); // keep routers alive until here
        endpoint_a.close().await;
        endpoint_b.close().await;
    }

    #[tokio::test]
    async fn reconnects_after_a_disconnect_and_keeps_delivering_messages() {
        let (endpoint_a, _router_a, direct_a, mut events_a, lookup_a) = spawn_direct(None).await;
        let key_b = SecretKey::generate();
        let (endpoint_b1, router_b1, _direct_b1, mut events_b1, lookup_b1) =
            spawn_direct(Some(key_b.clone())).await;
        lookup_a.add_endpoint_info(endpoint_b1.addr());
        lookup_b1.add_endpoint_info(endpoint_a.addr());

        let b_id = *endpoint_b1.id().as_bytes();
        direct_a.send(b_id, sample_message("first"));
        recv_matching(&mut events_b1, |e| matches!(e, NetEvent::Received(..))).await;

        // Simulate B going offline: tear its router and endpoint down
        // entirely.
        router_b1.shutdown().await.unwrap();
        endpoint_b1.close().await;
        recv_matching(&mut events_a, |e| matches!(e, NetEvent::PeerLeft(..))).await;

        // Bring B back as the *same* identity (same `SecretKey` ->  same
        // `EndpointId`) at a fresh address, which A has to re-learn --
        // standing in for discovery re-resolving a peer's address in
        // production.
        let (endpoint_b2, _router_b2, _direct_b2, mut events_b2, lookup_b2) =
            spawn_direct(Some(key_b)).await;
        lookup_a.add_endpoint_info(endpoint_b2.addr());
        lookup_b2.add_endpoint_info(endpoint_a.addr());

        direct_a.send(b_id, sample_message("second"));
        recv_matching(&mut events_a, |e| matches!(e, NetEvent::PeerJoined(..))).await;
        let received = recv_matching(&mut events_b2, |e| matches!(e, NetEvent::Received(..))).await;
        match received {
            NetEvent::Received(_, message) => assert_eq!(message.text, "second"),
            _ => unreachable!(),
        }

        endpoint_a.close().await;
        endpoint_b2.close().await;
    }

    #[tokio::test]
    async fn close_lets_a_later_send_start_a_fresh_actor() {
        let (endpoint_a, _router_a, direct_a, _events_a, lookup_a) = spawn_direct(None).await;
        let (endpoint_b, _router_b, _direct_b, mut events_b, lookup_b) = spawn_direct(None).await;
        lookup_a.add_endpoint_info(endpoint_b.addr());
        lookup_b.add_endpoint_info(endpoint_a.addr());

        let b_id = *endpoint_b.id().as_bytes();
        direct_a.send(b_id, sample_message("first"));
        recv_matching(&mut events_b, |e| matches!(e, NetEvent::Received(..))).await;

        direct_a.close(b_id);
        direct_a.send(b_id, sample_message("after close"));

        // A fresh actor was spawned by the second `send` (its id was
        // forgotten by `close`), so this should reconnect and deliver
        // normally -- `close` stops the *old* actor, it doesn't blackhole
        // the peer forever.
        let received = recv_matching(&mut events_b, |e| matches!(e, NetEvent::Received(..))).await;
        match received {
            NetEvent::Received(_, message) => assert_eq!(message.text, "after close"),
            _ => unreachable!(),
        }

        endpoint_a.close().await;
        endpoint_b.close().await;
    }
}
