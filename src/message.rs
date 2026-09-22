//! Wire format for chat messages exchanged over gossip.

use serde::{Deserialize, Serialize};

/// A single chat message, as it travels over the wire and is stored locally.
///
/// Kept small and versioned from day one -- see concept.md's "Message wire
/// format". Dedupe/ordering helpers land here once gossip is wired up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Wire format version.
    pub v: u8,
    /// Random message id, used for local dedupe.
    pub id: u64,
    /// Sender's NodeId. Embedded so messages can be verified/displayed
    /// without an extra lookup, even though gossip also gives you this at
    /// delivery.
    pub sender: [u8; 32],
    /// Unix timestamp in milliseconds.
    pub ts_unix_ms: u64,
    /// Message body.
    pub text: String,
}

/// Envelope for everything broadcast over a channel's gossip topic.
///
/// Wrapping `ChatMessage` here (rather than sending it bare, as earlier
/// build-order steps did) makes room for `Announce` without disturbing
/// `ChatMessage`'s own `v` field, which versions the message struct itself,
/// not the gossip envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GossipPayload {
    /// A chat message, exactly as sent by earlier build-order steps.
    Chat(ChatMessage),
    /// A history-sync announcement -- see `HistoryAnnounce`.
    Announce(HistoryAnnounce),
}

/// Broadcast to direct neighbors whenever a channel gains one (see net.rs's
/// handling of `iroh_gossip`'s `NeighborUp`): advertises the sender's current
/// history root hash for that channel, so a peer missing messages can fetch
/// them via `iroh-blobs` -- concept.md's "Persistence & offline history"
/// section describes this as asking a peer "for their log's latest hash and
/// pull the delta". See backfill.rs for how the root hash is built and used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryAnnounce {
    /// The announcing peer's own NodeId, so a recipient knows who to fetch
    /// from directly -- gossip's own `delivered_from` is only the relaying
    /// neighbor, not necessarily the announcement's author.
    pub sender: [u8; 32],
    /// Root hash of the sender's current history manifest for this channel.
    pub root: iroh_blobs::Hash,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_chat() -> ChatMessage {
        ChatMessage {
            v: 1,
            id: 42,
            sender: [3; 32],
            ts_unix_ms: 1000,
            text: "hi".to_string(),
        }
    }

    #[test]
    fn chat_payload_round_trips_through_postcard() {
        let payload = GossipPayload::Chat(sample_chat());
        let bytes = postcard::to_stdvec(&payload).unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn announce_payload_round_trips_through_postcard() {
        let payload = GossipPayload::Announce(HistoryAnnounce {
            sender: [7; 32],
            root: iroh_blobs::Hash::new(b"some history manifest"),
        });
        let bytes = postcard::to_stdvec(&payload).unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn chat_and_announce_payloads_are_distinguishable() {
        let chat_bytes = postcard::to_stdvec(&GossipPayload::Chat(sample_chat())).unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&chat_bytes).unwrap();
        assert!(matches!(decoded, GossipPayload::Chat(_)));
    }
}
