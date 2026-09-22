//! Invite ticket encode/decode.
//!
//! A ticket bundles a channel name and one peer's address into a single
//! string a friend can paste in to join -- see concept.md's "Identity &
//! channels" section. The channel's gossip topic is derived deterministically
//! from its name (see `net::topic_for_name`), so the ticket doesn't need to
//! carry the topic separately -- there's no way for the two to disagree.
//! Built on the `iroh-tickets` crate's `Ticket` trait, which handles the
//! base32 string round trip.

use iroh::EndpointAddr;
use iroh_tickets::{ParseError, Ticket};
use serde::{Deserialize, Serialize};

/// A channel name plus one peer's address, so a single pasted string is
/// enough to join that peer's gossip swarm for that channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelTicket {
    pub name: String,
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
}
