//! Device pairing: a one-time, direct exchange that lets a brand-new
//! device receive the shared user key and every known channel from an
//! already-initialized one, so it starts out recognized as the same
//! person (`crate::message::DeviceCert`) and already joined to
//! everything -- see concept.md's "Identity & channels".
//!
//! Mirrors `backfill.rs`'s relationship to `net.rs`: this module owns its
//! own protocol handler and in-memory state, and `net.rs` just registers
//! it on the shared `Router` alongside gossip and blobs.
//!
//! Redeeming a ticket (`bootstrap`) only ever happens before the normal
//! event loop starts, via the `--pair <ticket>` CLI flag (see main.rs) --
//! by the time a live command could run, this device would already have
//! generated its own, unrelated user key and an un-shared "general"
//! channel, which pairing would then have to unwind. Showing a ticket
//! (`PairingProtocol::create_ticket`, via the live `/pair` command) stays
//! a normal in-session action, since that's read-only.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::Context;
use iroh::endpoint::{Connection, presets};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr, NET_REPORT_TIMEOUT, SecretKey};
use iroh_tickets::Ticket;
use serde::{Deserialize, Serialize};
use tokio::time::timeout;
use tracing::warn;

use crate::channel_registry::ChannelRegistry;
use crate::identity;
use crate::ticket::{PairingTicket, RoomSecret};

/// ALPN for the pairing protocol -- registered on the same `Router` as
/// gossip and blobs (see `net.rs`), but only ever used for this one-shot
/// exchange, never for ongoing channel traffic.
pub const PAIRING_ALPN: &[u8] = b"leyline/pairing/1";

/// How long a pairing ticket stays redeemable after
/// `PairingProtocol::create_ticket` -- short enough that leaving a ticket
/// on screen for a while poses little extra risk, on top of it also being
/// single-use (see `PendingPairing`).
const PAIRING_TICKET_TTL: Duration = Duration::from_secs(300);

/// Generous upper bound on the redeemed secret's encoded size -- a
/// `[u8; 32]`'s postcard encoding is exactly 32 bytes, so this is a
/// guardrail against a broken or malicious peer streaming unbounded data,
/// not a real limit on the actual message.
const MAX_SECRET_BYTES: usize = 64;

/// Generous upper bound on the pairing response's encoded size: the user
/// key plus every known channel's name, secret, and peer addresses. 1 MiB
/// comfortably covers even a few hundred channels.
const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

/// What's handed to a device that successfully redeems a pairing ticket:
/// the shared user key and every channel the pairing device already
/// knows, in the same shape `net::Net::start` already accepts as
/// `known_channels` -- so the redeeming device can feed this straight
/// into its own startup without any translation.
#[derive(Serialize, Deserialize)]
pub struct PairingPayload {
    /// The shared user key's raw bytes -- not `iroh::SecretKey`'s own
    /// `Serialize` impl, matching how `identity.rs` already treats key
    /// material as raw bytes rather than depending on an external crate's
    /// wire format for something this sensitive.
    pub user_key: [u8; 32],
    pub channels: Vec<(String, RoomSecret, Vec<EndpointAddr>)>,
}

/// Redacts the raw user key so an accidental `{:?}` can't leak it --
/// mirrors `ticket::RoomSecret`'s and iroh's own `SecretKey`'s redacted
/// `Debug` impls, since this is credential material, not a display value.
impl std::fmt::Debug for PairingPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairingPayload")
            .field("user_key", &"..")
            .field("channels", &self.channels)
            .finish()
    }
}

/// A pairing ticket awaiting redemption: the secret it carries, when it
/// stops being valid, and the payload to hand back if it's redeemed in
/// time. Single-use -- `PairingProtocol::consume` clears this the moment
/// a matching secret is presented, valid or not, so a ticket can never be
/// redeemed twice regardless of outcome.
struct PendingPairing {
    secret: [u8; 32],
    expires_at: Instant,
    payload: PairingPayload,
}

impl std::fmt::Debug for PendingPairing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingPairing")
            .field("secret", &"..")
            .field("expires_at", &self.expires_at)
            .field("payload", &self.payload)
            .finish()
    }
}

/// Owns the one pairing ticket (if any) this device currently has
/// outstanding, and answers redemption attempts against it over
/// `PAIRING_ALPN`. A device only ever has one outstanding at a time --
/// creating a new one (`create_ticket`) replaces whatever was previously
/// pending, the same way re-running `/invite` for a channel just
/// re-shares its existing ticket rather than tracking more than one.
#[derive(Debug)]
pub struct PairingProtocol {
    pending: Mutex<Option<PendingPairing>>,
}

impl PairingProtocol {
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(None),
        }
    }

    /// Creates a new one-time pairing ticket good for `PAIRING_TICKET_TTL`,
    /// snapshotting `user_key` and `channels` to hand to whichever device
    /// redeems it -- see `PairingPayload`. `channels` is a snapshot at the
    /// moment this is called, the same way an invite ticket snapshots a
    /// channel's current secret: joining more channels afterward doesn't
    /// retroactively add them to an already-shown ticket. Replaces any
    /// previously-outstanding ticket.
    pub fn create_ticket(
        &self,
        addr: EndpointAddr,
        user_key: [u8; 32],
        channels: Vec<(String, RoomSecret, Vec<EndpointAddr>)>,
    ) -> PairingTicket {
        let secret: [u8; 32] = rand::random();
        *self.pending.lock().expect("pairing lock poisoned") = Some(PendingPairing {
            secret,
            expires_at: Instant::now() + PAIRING_TICKET_TTL,
            payload: PairingPayload { user_key, channels },
        });
        PairingTicket { addr, secret }
    }

    /// Checks `secret` against the currently-outstanding ticket (if any),
    /// consuming it either way -- so a ticket can be redeemed at most
    /// once, valid or not. `None` if there's no outstanding ticket, it
    /// expired, or `secret` doesn't match.
    fn consume(&self, secret: [u8; 32]) -> Option<PairingPayload> {
        let mut pending = self.pending.lock().expect("pairing lock poisoned");
        let candidate = pending.take()?;
        if candidate.secret == secret && candidate.expires_at > Instant::now() {
            Some(candidate.payload)
        } else {
            None
        }
    }
}

impl ProtocolHandler for PairingProtocol {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let (mut send, mut recv) = connection
            .accept_bi()
            .await
            .map_err(AcceptError::from_err)?;
        let secret_bytes = recv
            .read_to_end(MAX_SECRET_BYTES)
            .await
            .map_err(AcceptError::from_err)?;
        let secret: [u8; 32] =
            postcard::from_bytes(&secret_bytes).map_err(AcceptError::from_err)?;

        let Some(payload) = self.consume(secret) else {
            connection.close(1u32.into(), b"invalid or expired pairing secret");
            return Ok(());
        };

        let bytes = postcard::to_stdvec(&payload).map_err(AcceptError::from_err)?;
        send.write_all(&bytes)
            .await
            .map_err(AcceptError::from_err)?;
        send.finish().map_err(AcceptError::from_err)?;
        connection.closed().await;
        Ok(())
    }
}

/// Dials `ticket`'s address over `PAIRING_ALPN`, presents its secret, and
/// returns whatever the other side hands back -- the client half of the
/// exchange `PairingProtocol::accept` serves.
async fn redeem(endpoint: &Endpoint, ticket: &PairingTicket) -> anyhow::Result<PairingPayload> {
    let connection = endpoint
        .connect(ticket.addr.clone(), PAIRING_ALPN)
        .await
        .context("failed to connect to the pairing device")?;
    let (mut send, mut recv) = connection
        .open_bi()
        .await
        .context("failed to open a pairing stream")?;

    let secret_bytes =
        postcard::to_stdvec(&ticket.secret).context("failed to encode the pairing secret")?;
    send.write_all(&secret_bytes)
        .await
        .context("failed to send the pairing secret")?;
    send.finish()
        .context("failed to finish the pairing stream")?;

    let response = recv
        .read_to_end(MAX_PAYLOAD_BYTES)
        .await
        .context("failed to read the pairing response")?;
    postcard::from_bytes(&response).context("pairing device sent an invalid response")
}

/// Redeems `ticket` on a temporary endpoint bound with `device_key` (this
/// device's own, already loaded/generated identity -- see `identity.rs`),
/// then persists the received user key to `user_key_path` and seeds
/// `registry` with every received channel, so the normal startup path
/// (main.rs) picks both up immediately afterward exactly as if this
/// device had been running all along.
///
/// Refuses -- rather than silently overwriting -- if `user_key_path`
/// already exists: `--pair` is only for bootstrapping a device that has
/// never been paired before, not for re-pairing one that already has its
/// own identity and channels.
pub async fn bootstrap(
    device_key: &SecretKey,
    ticket: &str,
    user_key_path: &Path,
    registry: &mut ChannelRegistry,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !user_key_path.exists(),
        "this device already has a user key at {} -- --pair is only for a brand-new device",
        user_key_path.display()
    );
    let ticket = PairingTicket::decode_string(ticket).context("invalid --pair ticket")?;

    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(device_key.clone())
        .bind()
        .await
        .context("failed to bind a temporary endpoint for pairing")?;
    if timeout(Duration::from_secs(NET_REPORT_TIMEOUT), endpoint.online())
        .await
        .is_err()
    {
        warn!("no relay reachable yet; attempting to redeem the pairing ticket anyway");
    }
    let result = redeem(&endpoint, &ticket).await;
    endpoint.close().await;
    let payload = result?;

    identity::persist(user_key_path, &SecretKey::from_bytes(&payload.user_key))
        .context("failed to persist the received user key")?;
    for (name, secret, peers) in payload.channels {
        registry.record_channel(&name, secret)?;
        for peer in peers {
            registry.record_peer(&name, peer)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Arc;

    use iroh::TransportAddr;

    use super::*;

    fn sample_addr() -> EndpointAddr {
        EndpointAddr::from_parts(
            iroh::SecretKey::generate().public(),
            [TransportAddr::Ip(SocketAddr::from(([127, 0, 0, 1], 4242)))],
        )
    }

    #[test]
    fn consume_succeeds_with_the_matching_secret() {
        let pairing = PairingProtocol::new();
        let ticket = pairing.create_ticket(sample_addr(), [1; 32], vec![]);
        assert!(pairing.consume(ticket.secret).is_some());
    }

    #[test]
    fn consume_fails_with_the_wrong_secret() {
        let pairing = PairingProtocol::new();
        let _ticket = pairing.create_ticket(sample_addr(), [1; 32], vec![]);
        assert!(pairing.consume([0; 32]).is_none());
    }

    #[test]
    fn a_ticket_can_only_be_consumed_once() {
        let pairing = PairingProtocol::new();
        let ticket = pairing.create_ticket(sample_addr(), [1; 32], vec![]);
        assert!(pairing.consume(ticket.secret).is_some());
        assert!(
            pairing.consume(ticket.secret).is_none(),
            "a second redemption of the same secret must fail"
        );
    }

    #[test]
    fn creating_a_new_ticket_invalidates_the_previous_one() {
        let pairing = PairingProtocol::new();
        let first = pairing.create_ticket(sample_addr(), [1; 32], vec![]);
        let _second = pairing.create_ticket(sample_addr(), [1; 32], vec![]);
        assert!(
            pairing.consume(first.secret).is_none(),
            "only the most recently created ticket should be redeemable"
        );
    }

    #[test]
    fn consume_returns_the_snapshotted_payload() {
        let pairing = PairingProtocol::new();
        let channels = vec![("general".to_string(), RoomSecret::generate(), vec![])];
        let ticket = pairing.create_ticket(sample_addr(), [7; 32], channels);
        let payload = pairing.consume(ticket.secret).unwrap();
        assert_eq!(payload.user_key, [7; 32]);
        assert_eq!(payload.channels.len(), 1);
        assert_eq!(payload.channels[0].0, "general");
    }

    // The tests below exercise the real protocol end to end over loopback
    // (two actual `Endpoint`s, a real `Router`, a real QUIC connection) --
    // unlike the rest of this module's tests, which only exercise
    // `PairingProtocol`'s in-memory state. This is the strongest
    // evidence short of a real two-device pairing that `redeem` and
    // `PairingProtocol::accept` actually agree on the wire.

    async fn spawn_pairing_server() -> (iroh::protocol::Router, Arc<PairingProtocol>) {
        let endpoint = Endpoint::bind(presets::Minimal).await.unwrap();
        let pairing = Arc::new(PairingProtocol::new());
        let router = iroh::protocol::Router::builder(endpoint)
            .accept(PAIRING_ALPN, pairing.clone())
            .spawn();
        (router, pairing)
    }

    #[tokio::test]
    async fn redeem_over_a_real_connection_returns_the_exact_payload() {
        let (router, pairing) = spawn_pairing_server().await;
        let channels = vec![("general".to_string(), RoomSecret::generate(), vec![])];
        let ticket = pairing.create_ticket(router.endpoint().addr(), [9; 32], channels);

        let client = Endpoint::bind(presets::Minimal).await.unwrap();
        let payload = redeem(&client, &ticket).await.unwrap();

        assert_eq!(payload.user_key, [9; 32]);
        assert_eq!(payload.channels.len(), 1);
        assert_eq!(payload.channels[0].0, "general");

        client.close().await;
        router.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn redeeming_the_same_real_ticket_twice_fails_the_second_time() {
        let (router, pairing) = spawn_pairing_server().await;
        let ticket = pairing.create_ticket(router.endpoint().addr(), [1; 32], vec![]);

        let client = Endpoint::bind(presets::Minimal).await.unwrap();
        assert!(redeem(&client, &ticket).await.is_ok());
        assert!(
            redeem(&client, &ticket).await.is_err(),
            "redeeming an already-used ticket a second time must fail"
        );

        client.close().await;
        router.shutdown().await.unwrap();
    }
}
