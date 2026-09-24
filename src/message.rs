//! Wire format for chat messages exchanged over gossip.

use std::time::{SystemTime, UNIX_EPOCH};

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
    /// delivery anyway.
    pub sender: [u8; 32],
    /// Unix timestamp in milliseconds.
    pub ts_unix_ms: u64,
    /// Message body. Empty for a captionless file share (see `attachment`).
    pub text: String,
    /// A file shared via `/send`, if any -- see features.md's "File
    /// sharing" idea. Added behind the `v: 2` bump; `decode` below still
    /// loads messages persisted before this field existed.
    pub attachment: Option<FileAttachment>,
}

impl ChatMessage {
    /// Decodes a `ChatMessage` from postcard bytes, falling back to the
    /// pre-attachment (`v: 1`) shape for messages persisted or backfilled
    /// before `attachment` existed.
    ///
    /// postcard encodes structs positionally with no field names, so a
    /// plain `postcard::from_bytes::<ChatMessage>` fails outright on that
    /// older, shorter shape instead of defaulting the missing field --
    /// `storage::MessageStore::load` and `backfill::BackfillStore::fetch`
    /// call this instead, so a channel's history persisted before this
    /// upgrade still loads (just without an attachment, which it never had).
    pub fn decode(bytes: &[u8]) -> Result<ChatMessage, postcard::Error> {
        if let Ok(message) = postcard::from_bytes::<ChatMessage>(bytes) {
            return Ok(message);
        }
        postcard::from_bytes::<ChatMessageV1>(bytes).map(ChatMessageV1::into_current)
    }
}

/// `ChatMessage`'s shape before file attachments were added -- kept only so
/// `ChatMessage::decode` can still load messages persisted under the older,
/// shorter format. Never constructed directly otherwise (the `Serialize`
/// half only exists so tests can encode an old-shape record to decode).
#[derive(Serialize, Deserialize)]
struct ChatMessageV1 {
    v: u8,
    id: u64,
    sender: [u8; 32],
    ts_unix_ms: u64,
    text: String,
}

impl ChatMessageV1 {
    fn into_current(self) -> ChatMessage {
        ChatMessage {
            v: self.v,
            id: self.id,
            sender: self.sender,
            ts_unix_ms: self.ts_unix_ms,
            text: self.text,
            attachment: None,
        }
    }
}

/// A file shared with a channel via `/send` -- see features.md's "File
/// sharing" idea. `hash` addresses the content in the local iroh-blobs
/// store that also backs history backfill (see backfill.rs); receiving a
/// `ChatMessage` with an attachment never fetches the bytes automatically --
/// that only happens on an explicit `/save`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileAttachment {
    /// As typed in `/send <path>`, reduced to just its final path
    /// component -- see `crate::files::sanitize_filename`, applied again
    /// on the receiving end since a peer's filename is untrusted.
    pub filename: String,
    /// Size in bytes, shown before deciding whether to `/save`.
    pub size: u64,
    /// Content hash in the shared iroh-blobs store.
    pub hash: iroh_blobs::Hash,
}

/// Current time as Unix milliseconds -- shared by every place that composes
/// a new `ChatMessage` (`app::compose_message`, `net::Net::send_file`).
pub(crate) fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
    /// A broadcast-nickname announcement -- see `IdentityAnnounce`.
    Identity(IdentityAnnounce),
}

/// Flooded to the whole channel whenever a channel gains a gossip neighbor
/// (see net.rs's handling of `iroh_gossip`'s `NeighborUp`), and again
/// periodically regardless of neighbor churn (see main.rs's history-announce
/// heartbeat): advertises the sender's current history root hash for that
/// channel, so any peer missing messages -- not only the sender's direct
/// gossip neighbors -- can fetch them via `iroh-blobs`. concept.md's
/// "Persistence & history backfill" section describes this as asking a peer
/// "for their log's latest hash and pull the delta". See backfill.rs for how
/// the root hash is built and used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryAnnounce {
    /// The announcing peer's own NodeId, so a recipient knows who to fetch
    /// from directly -- gossip's own `delivered_from` is only the relaying
    /// neighbor, not necessarily the announcement's author.
    pub sender: [u8; 32],
    /// Root hash of the sender's current history manifest for this channel.
    pub root: iroh_blobs::Hash,
}

/// Broadcast to every joined channel when `/nick` is run, and re-sent to
/// direct neighbors whenever a channel gains one (see net.rs's handling of
/// `iroh_gossip`'s `NeighborUp`), the same way `HistoryAnnounce` is --
/// otherwise a peer who joins after you set your nickname would never learn
/// it. Convenient, but spoofable -- nothing stops two peers both claiming
/// the same nickname -- so `AppState::display_name` shows it alongside the
/// sender's id rather than in place of it, until a local petname (see
/// `contacts.rs`) is pinned for that id. See features.md's "Broadcast
/// nicknames".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityAnnounce {
    /// The announcing peer's own NodeId.
    pub sender: [u8; 32],
    /// The chosen display nickname, as typed after `/nick`.
    pub nickname: String,
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
            attachment: None,
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
    fn identity_payload_round_trips_through_postcard() {
        let payload = GossipPayload::Identity(IdentityAnnounce {
            sender: [5; 32],
            nickname: "alice".to_string(),
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

    #[test]
    fn chat_and_identity_payloads_are_distinguishable() {
        let identity_bytes = postcard::to_stdvec(&GossipPayload::Identity(IdentityAnnounce {
            sender: [5; 32],
            nickname: "alice".to_string(),
        }))
        .unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&identity_bytes).unwrap();
        assert!(matches!(decoded, GossipPayload::Identity(_)));
    }

    #[test]
    fn decode_round_trips_a_message_with_an_attachment() {
        let message = ChatMessage {
            v: 2,
            id: 7,
            sender: [4; 32],
            ts_unix_ms: 2000,
            text: String::new(),
            attachment: Some(FileAttachment {
                filename: "report.pdf".to_string(),
                size: 1234,
                hash: iroh_blobs::Hash::new(b"report contents"),
            }),
        };
        let bytes = postcard::to_stdvec(&message).unwrap();
        assert_eq!(ChatMessage::decode(&bytes).unwrap(), message);
    }

    #[test]
    fn decode_loads_a_message_persisted_before_attachments_existed() {
        // Simulates a record written to disk (or backfilled) before
        // `attachment` was added to `ChatMessage` -- decode must still
        // succeed, with `attachment` defaulting to `None`, rather than
        // failing outright the way a plain `postcard::from_bytes` would.
        let old = ChatMessageV1 {
            v: 1,
            id: 99,
            sender: [8; 32],
            ts_unix_ms: 500,
            text: "from before the upgrade".to_string(),
        };
        let bytes = postcard::to_stdvec(&old).unwrap();

        let decoded = ChatMessage::decode(&bytes).unwrap();

        assert_eq!(decoded.v, 1);
        assert_eq!(decoded.id, 99);
        assert_eq!(decoded.sender, [8; 32]);
        assert_eq!(decoded.ts_unix_ms, 500);
        assert_eq!(decoded.text, "from before the upgrade");
        assert_eq!(decoded.attachment, None);
    }

    #[test]
    fn decode_rejects_genuinely_malformed_bytes() {
        assert!(ChatMessage::decode(b"not a chat message").is_err());
    }
}
