//! Wire format for chat messages exchanged over gossip.

use std::time::{SystemTime, UNIX_EPOCH};

use iroh::{EndpointAddr, PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};

use crate::ticket::RoomSecret;

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
    /// The id of the message this one is replying to, if any -- see
    /// features.md's "Replies" idea. Added behind the `v: 3` bump;
    /// `decode` below still loads messages persisted before this field
    /// existed. Never verified to actually resolve -- the replied-to
    /// message may have expired from scrollback, never arrived, or never
    /// existed at all, so renderers (see `ui::reply_preview_spans`) must
    /// handle a dangling id gracefully rather than assuming it resolves.
    pub reply_to: Option<u64>,
}

impl ChatMessage {
    /// Decodes a `ChatMessage` from postcard bytes, falling back to older
    /// shapes for messages persisted or backfilled before this build's
    /// fields existed: `v: 2` (pre-`reply_to`) then `v: 1` (pre-`attachment`
    /// too).
    ///
    /// postcard encodes structs positionally with no field names, so a
    /// plain `postcard::from_bytes::<ChatMessage>` fails outright on an
    /// older, shorter shape instead of defaulting the missing field(s) --
    /// `storage::MessageStore::load` and `backfill::BackfillStore::fetch`
    /// call this instead, so a channel's history persisted before either
    /// upgrade still loads (just without whatever field it predates).
    pub fn decode(bytes: &[u8]) -> Result<ChatMessage, postcard::Error> {
        if let Ok(message) = postcard::from_bytes::<ChatMessage>(bytes) {
            return Ok(message);
        }
        if let Ok(message) = postcard::from_bytes::<ChatMessageV2>(bytes) {
            return Ok(message.into_current());
        }
        postcard::from_bytes::<ChatMessageV1>(bytes).map(ChatMessageV1::into_current)
    }
}

/// `ChatMessage`'s shape before replies were added -- kept only so
/// `ChatMessage::decode` can still load messages persisted under this
/// older, shorter format. Never constructed directly otherwise (the
/// `Serialize` half only exists so tests can encode an old-shape record to
/// decode).
#[derive(Serialize, Deserialize)]
struct ChatMessageV2 {
    v: u8,
    id: u64,
    sender: [u8; 32],
    ts_unix_ms: u64,
    text: String,
    attachment: Option<FileAttachment>,
}

impl ChatMessageV2 {
    fn into_current(self) -> ChatMessage {
        ChatMessage {
            v: self.v,
            id: self.id,
            sender: self.sender,
            ts_unix_ms: self.ts_unix_ms,
            text: self.text,
            attachment: self.attachment,
            reply_to: None,
        }
    }
}

/// `ChatMessage`'s shape before file attachments (or replies) were added --
/// kept only so `ChatMessage::decode` can still load messages persisted
/// under the oldest, shortest format. Never constructed directly otherwise
/// (the `Serialize` half only exists so tests can encode an old-shape
/// record to decode).
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
            reply_to: None,
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
    /// A build-version announcement -- see `VersionAnnounce`.
    Version(VersionAnnounce),
    /// A device-to-user binding -- see `DeviceCert`.
    Device(DeviceCert),
    /// Another of our own devices announcing it joined a channel -- see
    /// `ChannelSyncAnnounce`.
    ChannelJoined(ChannelSyncAnnounce),
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

/// Broadcast to every joined channel once we know a neighbor is listening
/// (see net.rs's handling of `iroh_gossip`'s `NeighborUp`, the same way
/// `HistoryAnnounce`/`IdentityAnnounce` are): our own build's version and
/// git commit, so a peer running an older build can be told about it (see
/// `crate::version::is_newer`). Unlike `IdentityAnnounce`, this is never
/// user-chosen -- it's always known from the moment we start, so there's
/// no "unset" state to track.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionAnnounce {
    /// The announcing peer's own NodeId.
    pub sender: [u8; 32],
    /// The announcing peer's `crate::version::VERSION`.
    pub version: String,
    /// The announcing peer's `crate::version::GIT_HASH` -- informational
    /// only (never compared), since commit hashes have no inherent
    /// ordering.
    pub git_hash: String,
}

/// Binds a device's own `EndpointId` to a longer-lived user key, so a peer
/// who sees certificates for two different device ids signed by the same
/// user key can treat them as the same person -- see concept.md's
/// "Identity & channels". Broadcast as `GossipPayload::Device`, re-sent on
/// `NeighborUp` the same way `IdentityAnnounce`/`VersionAnnounce` are (see
/// `net::Net::announce_device`). Every device has one from first run
/// onward (see `identity.rs`), so there's no "no cert" state for *our own*
/// device to handle -- only a peer we haven't heard from yet, or one on a
/// build that predates this, lacks one (`AppState::canonical_id` falls
/// back to treating such a peer's device id as its own identity,
/// unchanged).
///
/// Never expires and is never revoked over the network: removing a device
/// is a local, best-effort decision, and real removal is the same
/// "rotate the secret" story LEY-11 already documents for channels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCert {
    /// The device's own EndpointId -- the key that actually
    /// authenticates/encrypts its connections, same as everywhere else in
    /// this wire format.
    pub device_id: [u8; 32],
    /// The user key's public half. Two certs with the same `user_id` (and
    /// a valid `signature` each) belong to the same person.
    pub user_id: [u8; 32],
    /// When this device was bound to the user key, for display/debugging
    /// only -- never checked for expiry, and not persisted anywhere, so it
    /// simply reflects whenever this process last started up.
    pub issued_at_unix_ms: u64,
    /// `user_id`'s signature over this cert's other three fields.
    pub signature: Signature,
}

impl DeviceCert {
    /// Builds and signs a fresh certificate binding `device_id` (this
    /// device's own EndpointId) to `user_key`.
    pub fn new(device_id: [u8; 32], user_key: &SecretKey, issued_at_unix_ms: u64) -> Self {
        let user_id = *user_key.public().as_bytes();
        let signature = user_key.sign(&Self::signed_bytes(device_id, user_id, issued_at_unix_ms));
        Self {
            device_id,
            user_id,
            issued_at_unix_ms,
            signature,
        }
    }

    /// Checks this cert's `signature` against its own `user_id`. `false`
    /// for a tampered field, a malformed `user_id`, or a signature made by
    /// a different key -- callers (`net::forward_events`) drop a cert that
    /// fails this rather than ever recording it.
    pub fn is_valid(&self) -> bool {
        let Ok(user_key) = PublicKey::from_bytes(&self.user_id) else {
            return false;
        };
        let bytes = Self::signed_bytes(self.device_id, self.user_id, self.issued_at_unix_ms);
        user_key.verify(&bytes, &self.signature).is_ok()
    }

    /// The bytes `new`/`is_valid` sign/verify: every field except
    /// `signature` itself, postcard-encoded.
    fn signed_bytes(device_id: [u8; 32], user_id: [u8; 32], issued_at_unix_ms: u64) -> Vec<u8> {
        postcard::to_stdvec(&(device_id, user_id, issued_at_unix_ms))
            .expect("postcard serialization is infallible")
    }
}

/// Broadcast only on the private per-user device-sync channel
/// (`net::DEVICE_SYNC_CHANNEL`) whenever a device joins a channel its
/// sibling devices might not know about yet, so they can auto-join it too
/// -- see `net::Net::sync_channel`/`announce_channel_joined` and
/// concept.md's "Identity & channels". Never meaningful on any other
/// channel: acting on one received elsewhere would let an unrelated peer
/// direct this device to join arbitrary channels, so `net::forward_events`
/// tags every `NetEvent` with the channel it actually arrived on, and
/// `Net::sync_channel` refuses to act unless that tag is
/// `DEVICE_SYNC_CHANNEL`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelSyncAnnounce {
    /// The announcing device's own EndpointId -- checked against our own
    /// id (so we don't try to rejoin a channel we just announced
    /// ourselves) and against any locally forgotten device ids (see
    /// `crate::forgotten_devices`) before ever being acted on.
    pub sender: [u8; 32],
    /// The joined channel's display name.
    pub name: String,
    /// The joined channel's room secret -- lets a sibling device join
    /// directly, with no ticket exchange of its own.
    pub secret: RoomSecret,
    /// Address hints for reaching the channel's mesh, so a sibling device
    /// has something concrete to bootstrap from. Always includes at least
    /// the announcing device's own address (see `announce_channel_joined`).
    pub peers: Vec<EndpointAddr>,
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
            reply_to: None,
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
    fn version_payload_round_trips_through_postcard() {
        let payload = GossipPayload::Version(VersionAnnounce {
            sender: [6; 32],
            version: "1.2.3".to_string(),
            git_hash: "abc1234".to_string(),
        });
        let bytes = postcard::to_stdvec(&payload).unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn chat_and_version_payloads_are_distinguishable() {
        let version_bytes = postcard::to_stdvec(&GossipPayload::Version(VersionAnnounce {
            sender: [6; 32],
            version: "1.2.3".to_string(),
            git_hash: "abc1234".to_string(),
        }))
        .unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&version_bytes).unwrap();
        assert!(matches!(decoded, GossipPayload::Version(_)));
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
            reply_to: None,
        };
        let bytes = postcard::to_stdvec(&message).unwrap();
        assert_eq!(ChatMessage::decode(&bytes).unwrap(), message);
    }

    #[test]
    fn decode_round_trips_a_message_with_a_reply() {
        let message = ChatMessage {
            v: 3,
            id: 8,
            sender: [4; 32],
            ts_unix_ms: 2001,
            text: "sounds good".to_string(),
            attachment: None,
            reply_to: Some(7),
        };
        let bytes = postcard::to_stdvec(&message).unwrap();
        assert_eq!(ChatMessage::decode(&bytes).unwrap(), message);
    }

    #[test]
    fn decode_loads_a_message_persisted_before_attachments_existed() {
        // Simulates a record written to disk (or backfilled) before
        // `attachment` was added to `ChatMessage` -- decode must still
        // succeed, with `attachment` (and `reply_to`) defaulting to `None`,
        // rather than failing outright the way a plain `postcard::from_bytes`
        // would.
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
        assert_eq!(decoded.reply_to, None);
    }

    #[test]
    fn decode_loads_a_message_persisted_before_replies_existed() {
        // Simulates a record written to disk (or backfilled) before
        // `reply_to` was added to `ChatMessage` -- decode must still
        // succeed, with `reply_to` defaulting to `None`, rather than falling
        // all the way back to the even-older `ChatMessageV1` shape (which
        // would also incorrectly drop the attachment).
        let old = ChatMessageV2 {
            v: 2,
            id: 100,
            sender: [9; 32],
            ts_unix_ms: 600,
            text: String::new(),
            attachment: Some(FileAttachment {
                filename: "notes.txt".to_string(),
                size: 42,
                hash: iroh_blobs::Hash::new(b"notes"),
            }),
        };
        let bytes = postcard::to_stdvec(&old).unwrap();

        let decoded = ChatMessage::decode(&bytes).unwrap();

        assert_eq!(decoded.v, 2);
        assert_eq!(decoded.id, 100);
        assert_eq!(decoded.reply_to, None);
        assert!(
            decoded.attachment.is_some(),
            "attachment must survive this fallback tier"
        );
    }

    #[test]
    fn decode_rejects_genuinely_malformed_bytes() {
        assert!(ChatMessage::decode(b"not a chat message").is_err());
    }

    fn sample_device_cert(user_key: &SecretKey, device_id: [u8; 32]) -> DeviceCert {
        DeviceCert::new(device_id, user_key, 1_000)
    }

    #[test]
    fn device_cert_is_valid_when_correctly_signed() {
        let user_key = SecretKey::generate();
        let cert = sample_device_cert(&user_key, [1; 32]);
        assert!(cert.is_valid());
    }

    #[test]
    fn device_cert_is_invalid_when_the_device_id_is_tampered_with() {
        let user_key = SecretKey::generate();
        let mut cert = sample_device_cert(&user_key, [1; 32]);
        cert.device_id = [2; 32];
        assert!(!cert.is_valid());
    }

    #[test]
    fn device_cert_is_invalid_for_a_signature_from_a_different_key() {
        let user_key = SecretKey::generate();
        let other_key = SecretKey::generate();
        let mut cert = sample_device_cert(&user_key, [1; 32]);
        // Swap in a different user's public id without re-signing --
        // simulates an attacker claiming someone else's user id.
        cert.user_id = *other_key.public().as_bytes();
        assert!(!cert.is_valid());
    }

    #[test]
    fn two_devices_signed_by_the_same_user_key_share_a_user_id() {
        let user_key = SecretKey::generate();
        let a = sample_device_cert(&user_key, [1; 32]);
        let b = sample_device_cert(&user_key, [2; 32]);
        assert_eq!(a.user_id, b.user_id);
        assert_ne!(a.device_id, b.device_id);
    }

    #[test]
    fn device_payload_round_trips_through_postcard() {
        let user_key = SecretKey::generate();
        let payload = GossipPayload::Device(sample_device_cert(&user_key, [1; 32]));
        let bytes = postcard::to_stdvec(&payload).unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn chat_and_device_payloads_are_distinguishable() {
        let user_key = SecretKey::generate();
        let device_bytes = postcard::to_stdvec(&GossipPayload::Device(sample_device_cert(
            &user_key, [1; 32],
        )))
        .unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&device_bytes).unwrap();
        assert!(matches!(decoded, GossipPayload::Device(_)));
    }

    fn sample_channel_sync_announce() -> ChannelSyncAnnounce {
        ChannelSyncAnnounce {
            sender: [5; 32],
            name: "project-x".to_string(),
            secret: crate::ticket::RoomSecret::generate(),
            peers: Vec::new(),
        }
    }

    #[test]
    fn channel_joined_payload_round_trips_through_postcard() {
        let payload = GossipPayload::ChannelJoined(sample_channel_sync_announce());
        let bytes = postcard::to_stdvec(&payload).unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(decoded, payload);
    }

    #[test]
    fn chat_and_channel_joined_payloads_are_distinguishable() {
        let bytes =
            postcard::to_stdvec(&GossipPayload::ChannelJoined(sample_channel_sync_announce()))
                .unwrap();
        let decoded: GossipPayload = postcard::from_bytes(&bytes).unwrap();
        assert!(matches!(decoded, GossipPayload::ChannelJoined(_)));
    }
}
