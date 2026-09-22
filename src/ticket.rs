//! Invite ticket encode/decode.
//!
//! A ticket bundles a channel's display name, its room secret, and one
//! peer's address into a single string a friend can paste in to join --
//! see concept.md's "Identity & channels" section. Built on the
//! `iroh-tickets` crate's `Ticket` trait, which handles the base32 string
//! round trip.

use std::fmt;

use iroh::EndpointAddr;
use iroh_tickets::{ParseError, Ticket};
use serde::{Deserialize, Serialize};

/// A room's private key material: 32 random bytes that determine its
/// gossip topic (see `net::topic_for_secret`) instead of its human-readable
/// name.
///
/// Knowing this -- not knowing the name -- is what it means to be let into
/// a room, so treat it like a password: it's generated fresh the moment a
/// room is created (never derived from a name or an identity, both of
/// which can be guessed or are already public), and it only ever reaches
/// someone else by riding along inside a `ChannelTicket`. This is also
/// what makes two rooms that happen to share a display name land on two
/// different topics instead of colliding.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoomSecret([u8; 32]);

impl RoomSecret {
    /// Generates a fresh, random room secret. Used the first time a
    /// channel is created locally -- either as the default channel on a
    /// genuinely first run, or via `/join <new-name>` -- never derived
    /// deterministically from anything, so it can't be recomputed by
    /// anyone who wasn't handed it.
    pub fn generate() -> Self {
        Self(rand::random())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Redacts the secret bytes so an accidental `{:?}` in a log line can't
/// leak it -- this is credential material, not a display value.
impl fmt::Debug for RoomSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RoomSecret(..)")
    }
}

/// A channel's display name plus its room secret and one peer's address,
/// so a single pasted string is enough to join that peer's gossip swarm
/// for that channel. `name` is a label only -- see `RoomSecret` and
/// `net::topic_for_secret` for why the actual swarm is keyed by `secret`,
/// not `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelTicket {
    pub name: String,
    pub secret: RoomSecret,
    pub addr: EndpointAddr,
}

impl Ticket for ChannelTicket {
    const KIND: &'static str = "leyline";

    fn encode_bytes(&self) -> Vec<u8> {
        postcard::to_stdvec(self).expect("postcard serialization is infallible")
    }

    fn decode_bytes(bytes: &[u8]) -> Result<Self, ParseError> {
        Ok(postcard::from_bytes(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use iroh::TransportAddr;

    use super::*;

    fn sample_ticket() -> ChannelTicket {
        let addr = EndpointAddr::from_parts(
            iroh::SecretKey::generate().public(),
            [TransportAddr::Ip(SocketAddr::from(([127, 0, 0, 1], 4242)))],
        );
        ChannelTicket {
            name: "general".to_string(),
            secret: RoomSecret::generate(),
            addr,
        }
    }

    #[test]
    fn round_trips_through_its_string_form() {
        let ticket = sample_ticket();
        let encoded = ticket.encode_string();
        assert!(encoded.starts_with("leyline"));

        let decoded = ChannelTicket::decode_string(&encoded).unwrap();
        assert_eq!(ticket, decoded);
    }

    #[test]
    fn rejects_garbage_input() {
        assert!(ChannelTicket::decode_string("not-a-ticket").is_err());
    }

    #[test]
    fn rejects_a_wrong_kind_prefix() {
        // A validly-formed ticket of a different kind, e.g. iroh-tickets'
        // own EndpointTicket, should not be mistaken for ours.
        let addr = EndpointAddr::from_parts(iroh::SecretKey::generate().public(), []);
        let other = iroh_tickets::endpoint::EndpointTicket::new(addr);
        assert!(ChannelTicket::decode_string(&other.encode_string()).is_err());
    }

    #[test]
    fn two_rooms_with_the_same_name_get_different_secrets() {
        // The whole point of `RoomSecret`: a display name is never the
        // credential, so two independently-created rooms that happen to
        // share one must not be joinable with each other's tickets.
        let a = sample_ticket();
        let mut b = sample_ticket();
        b.name = a.name.clone();
        assert_ne!(a.secret, b.secret);
    }
}
