//! iroh + iroh-gossip networking.
//!
//! Owns an `iroh::Endpoint` and a subscription per joined gossip topic --
//! one gossip task per channel, per concept.md's "Architecture at a glance"
//! section -- forwarding parsed `NetEvent`s, each tagged with the channel
//! name it came from, into the app's event loop over an `mpsc` channel.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Context;
use arboard::{Clipboard, ImageData};
use futures::StreamExt;
use image::{ImageFormat, RgbaImage};
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
    ChannelSyncAnnounce, ChatMessage, DeviceCert, FileAttachment, GossipPayload, HistoryAnnounce,
    IdentityAnnounce, VersionAnnounce, now_unix_ms,
};
use crate::pairing::{self, PairingProtocol};
use crate::ticket::{ChannelTicket, PairingTicket, RoomSecret};
use crate::version;

/// Simple guardrail against an accidental huge `/send` (e.g. a mistyped
/// path to a large video file): `BackfillStore::add_file` copies the file
/// into the local blob store, so a sent file's bytes are duplicated on
/// disk locally -- see net.rs's `send_file`.
const MAX_FILE_SIZE: u64 = 500 * 1024 * 1024;

/// The channel every instance joins on startup.
const DEFAULT_CHANNEL: &str = "general";

/// A private, per-user channel every device belonging to the same person
/// auto-joins (see `Net::start`), used only to broadcast
/// `crate::message::ChannelSyncAnnounce`s so sibling devices learn about a
/// newly-joined channel without needing to be re-paired -- see
/// `Net::sync_channel`/`announce_channel_joined` and concept.md's
/// "Identity & channels". Its `RoomSecret` is derived from the shared user
/// key (`RoomSecret::derive_from_user_key`), never generated or carried in
/// a ticket, so nothing needs to be transferred for every device to land
/// on the same topic.
///
/// Reserved and deliberately *not* an ordinary channel from the UI's
/// perspective: it is excluded from the `joined` list `Net::start`
/// returns, so it never becomes an `AppState` tab, is never printed as an
/// `/invite`-able ticket, and `Net::join` refuses to (re)join it by name
/// -- see each of those for why. A leaked ticket for it would let anyone
/// who redeemed it feed this device fabricated `ChannelSyncAnnounce`s, so
/// `Net::sync_channel` double-checks that one actually arrived on this
/// exact channel before ever acting on it.
pub const DEVICE_SYNC_CHANNEL: &str = "_devices";

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
    /// A channel finished subscribing via a DM-flagged invite ticket
    /// (`ticket::ChannelTicket::dm`) and is ready to send/receive on --
    /// the recipient side of a `/msg`-initiated DM (see
    /// `app::InputAction::StartDm`), so this side can also record the
    /// ticket's sharer as its DM partner for this channel
    /// (`crate::dm_registry`) and converge on reusing it, the same
    /// channel the initiator is already using for this pair. The
    /// initiator's own side never sees this variant -- a freshly created
    /// channel is joined by bare name, with no ticket to read a `dm` flag
    /// from, so it reports back as a plain `Joined` and main.rs correlates
    /// that back to a peer itself (see its `pending_dms` map). Carries the
    /// same activation responsibility `Joined` does
    /// (`app::AppState::handle_net_event`).
    DmJoined { channel: String, peer: [u8; 32] },
    /// A channel failed to join.
    JoinFailed(String, String),
    /// A peer announced its current history root hash for a channel -- see
    /// `crate::message::HistoryAnnounce` and backfill.rs.
    Announce(String, HistoryAnnounce),
    /// A peer announced (or re-announced) their broadcast nickname -- see
    /// `crate::message::IdentityAnnounce`.
    Identity(String, IdentityAnnounce),
    /// A peer announced (or re-announced) their build version -- see
    /// `crate::message::VersionAnnounce`.
    Version(String, VersionAnnounce),
    /// A peer announced (or re-announced) a certificate binding one of
    /// their device ids to a user key -- see `crate::message::DeviceCert`.
    /// Already verified (`DeviceCert::is_valid`) by the time this is
    /// emitted -- see `forward_events` -- so `app.rs` never has to.
    Device(String, DeviceCert),
    /// A sibling device announced joining a channel -- see
    /// `crate::message::ChannelSyncAnnounce`. Tagged with the channel this
    /// arrived on (like every other `NetEvent`) so `Net::sync_channel` can
    /// refuse to act on one that did not actually arrive on
    /// `DEVICE_SYNC_CHANNEL`.
    ChannelSync(String, ChannelSyncAnnounce),
    /// A channel learned via `DEVICE_SYNC_CHANNEL` finished subscribing --
    /// see `Net::join_known`. Deliberately distinct from `Joined`/
    /// `DmJoined`: both of those switch the active tab, since a user
    /// explicitly asked to go there, but a sibling device auto-joining a
    /// channel in the background must not yank focus away from whatever
    /// this device's user is actually looking at.
    ChannelSyncJoined(String),
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
    /// `/paste` found one or more real files on the OS clipboard (e.g. a
    /// Finder/Explorer copy) -- see `Net::paste`. Reported as paths, not
    /// already-imported attachments, so the caller can share each one via
    /// the exact same `Net::send_file` path `/send` uses -- this is what
    /// keeps a copied file's original bytes (including a real animated
    /// GIF) intact.
    ClipboardFiles(String, Vec<PathBuf>),
    /// `/paste` found rendered image pixels on the OS clipboard (e.g. a
    /// screenshot, or a browser's "Copy Image") and re-encoded them as PNG
    /// -- see `Net::paste` and `encode_png`. Always a single static frame:
    /// no OS clipboard image format carries multi-frame/animation data, so
    /// this is never a real animated GIF even if the source image was one
    /// -- only `ClipboardFiles` preserves that.
    ClipboardImage(String, String, Vec<u8>),
    /// `/paste` found no file or image on the clipboard, but did find
    /// plain text -- shared as an ordinary chat message, the same as if it
    /// had been typed and sent.
    ClipboardText(String, String),
    /// `/paste` couldn't find anything to share (an empty or unreadable
    /// clipboard, or the clipboard itself was unavailable) -- see
    /// `Net::paste`.
    PasteFailed(String),
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

/// A decoded ticket's payload, as returned by `parse_join_arg`: its room
/// secret, the sharer's address, and whether it's DM-flagged (see
/// `ticket::ChannelTicket::dm`).
type ParsedTicket = (RoomSecret, EndpointAddr, bool);

/// Interprets a `/join` argument as either a bare channel name or an
/// invite ticket.
///
/// If `arg` doesn't even have the ticket prefix, it's treated as a plain
/// channel name with no room secret yet -- see `Net::join`, which
/// generates a fresh one for a name it's never seen before. If it *does*
/// have the prefix but still fails to decode -- e.g. because it was
/// truncated when copied out of a narrow terminal -- that's reported as an
/// error instead of silently joining a bogus channel named after the
/// mangled string. The ticket's `dm` marker rides along too, so `Net::join`
/// can tell a DM invite apart from an ordinary one (see
/// `NetEvent::DmJoined`).
fn parse_join_arg(arg: &str) -> Result<(String, Option<ParsedTicket>), String> {
    match ChannelTicket::decode_string(arg) {
        Ok(ticket) => Ok((ticket.name, Some((ticket.secret, ticket.addr, ticket.dm)))),
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

/// Expands a `/send`-typed path for the local filesystem: a leading `~`
/// (bare, or `~/...`) to the user's home directory, and any relative path
/// to absolute. Both matter because the path is typed directly into
/// leyline's own input box, not a shell -- nothing else expands `~`
/// here -- and because iroh-blobs' importer (`BackfillStore::add_file`)
/// hard-requires an absolute path, rejecting a relative one outright.
/// Falls back to the path as typed if the home or current directory can't
/// be determined.
fn expand_path(path: &str) -> PathBuf {
    let expanded = if path == "~" {
        directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
    } else if let Some(rest) = path.strip_prefix("~/") {
        directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(rest))
    } else {
        None
    }
    .unwrap_or_else(|| PathBuf::from(path));

    std::path::absolute(&expanded).unwrap_or(expanded)
}

/// Encodes clipboard pixel data (`ImageData`'s raw RGBA8 buffer, row-major,
/// no padding -- see `Net::paste`) as a PNG, the `/paste` counterpart to
/// `expand_path`'s filesystem handling: clipboard image data has no
/// filename or original file format to preserve, so PNG (lossless,
/// universally viewable) is what actually gets shared, not the raw pixel
/// buffer itself. Always produces a single static frame -- `ImageData` has
/// no concept of multiple frames, so an animated image copied as rendered
/// pixels (as opposed to a file -- see `Net::paste`'s file-list check) can
/// never round-trip as a GIF through this path.
fn encode_png(image: ImageData) -> anyhow::Result<Vec<u8>> {
    let width = u32::try_from(image.width).context("clipboard image width overflowed u32")?;
    let height = u32::try_from(image.height).context("clipboard image height overflowed u32")?;
    let buffer = RgbaImage::from_raw(width, height, image.bytes.into_owned())
        .context("clipboard image dimensions didn't match its pixel buffer")?;
    let mut png_bytes = Vec::new();
    buffer
        .write_to(&mut std::io::Cursor::new(&mut png_bytes), ImageFormat::Png)
        .context("failed to encode clipboard image as PNG")?;
    Ok(png_bytes)
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
    /// Our current broadcast nickname, if one has ever been set via `/nick`
    /// -- re-sent to each channel's newly-up neighbors the same way a
    /// `HistoryAnnounce` is (see `announce_nickname`), so a peer who
    /// connects after we set it still learns it. Seeded from the previous
    /// session's persisted value at startup (see `Net::start` and
    /// `crate::settings::Settings::nickname`), unlike `nicknames` on
    /// `AppState`, which tracks *peers'* announced names and is never
    /// persisted.
    nickname: Mutex<Option<String>>,
    /// Our own endpoint id, as raw bytes so callers don't need to depend on
    /// iroh types.
    pub our_id: [u8; 32],
    /// This device's own certificate, binding `our_id` to the user key
    /// passed into `Net::start` -- see `crate::message::DeviceCert`.
    /// Computed once at startup and never changes for the life of this
    /// process; re-broadcast by `announce_device` the same way
    /// `nickname`/our build version already are.
    our_cert: DeviceCert,
    /// The shared user key's raw bytes, handed to a new device that
    /// redeems a pairing ticket (see `create_pairing_ticket` and
    /// `pairing.rs`). Raw bytes rather than `iroh::SecretKey`, matching
    /// how `identity.rs` already treats key material as plain bytes.
    user_key_bytes: [u8; 32],
    /// Registered on `router` under `pairing::PAIRING_ALPN`, and also kept
    /// here so `create_pairing_ticket` can produce tickets against the
    /// exact same pending-secret state the registered handler checks --
    /// mirrors how `gossip` is both registered on `router` and kept as its
    /// own field for later calls.
    pairing: Arc<PairingProtocol>,
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
    /// channel forwarding events to `events_tx`. `nickname` seeds our
    /// broadcast nickname from the previous session's persisted value (see
    /// `crate::settings::Settings::nickname`), if any, so it's ready to
    /// re-announce (via `announce_nickname`) as soon as a channel gains a
    /// neighbor, without needing `/nick` retyped first. Also registers the
    /// device-pairing protocol (`pairing::PairingProtocol`) on the same
    /// router, so `create_pairing_ticket`/`/pair` can bootstrap a
    /// brand-new device later. Returns the names of the channels joined,
    /// in join order, plus the name of the channel that should start
    /// active (the `--join` ticket's channel, if one was given and
    /// usable, else "general").
    pub async fn start(
        secret_key: SecretKey,
        user_key: SecretKey,
        join_ticket: Option<String>,
        known_channels: Vec<(String, RoomSecret, Vec<EndpointAddr>)>,
        nickname: Option<String>,
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
        let pairing = Arc::new(PairingProtocol::new());
        let router = Router::builder(endpoint)
            .accept(GOSSIP_ALPN, gossip.clone())
            .accept(iroh_blobs::ALPN, backfill.protocol_handler())
            .accept(pairing::PAIRING_ALPN, pairing.clone())
            .spawn();

        let our_cert = DeviceCert::new(our_id, &user_key, now_unix_ms());
        let user_key_bytes = user_key.to_bytes();

        let net = Self {
            router,
            gossip,
            address_lookup,
            channels: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
            backfill,
            nickname: Mutex::new(nickname),
            our_id,
            our_cert,
            user_key_bytes,
            pairing,
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
            if name.as_str() == DEFAULT_CHANNEL || name.as_str() == DEVICE_SYNC_CHANNEL {
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

        // Every device also joins its own private device-sync channel
        // (see `DEVICE_SYNC_CHANNEL`'s doc comment): its secret is always
        // derived from the shared user key, never looked up or generated,
        // and it is deliberately left out of `joined` so it never becomes
        // a visible tab or an `/invite`-able ticket below. Bootstrap peers
        // are still reused from a previous session exactly like any other
        // channel, so reconnecting to sibling devices gets faster over
        // time (see main.rs, which records learned peers for it the same
        // way it does for every other channel).
        let device_sync_secret = RoomSecret::derive_from_user_key(&user_key_bytes);
        let device_sync_bootstrap = known_channels
            .iter()
            .find(|(name, _, _)| name.as_str() == DEVICE_SYNC_CHANNEL)
            .map(|(_, _, addrs)| addrs.clone())
            .unwrap_or_default();
        net.subscribe_and_register(
            DEVICE_SYNC_CHANNEL,
            device_sync_secret,
            device_sync_bootstrap,
        )
        .await
        .context("failed to join device-sync channel")?;

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
    /// `NetEvent::Joined`/`DmJoined`/`JoinFailed`, so the caller's event
    /// loop never awaits network I/O directly. Reports `DmJoined` instead
    /// of `Joined` when `arg` decoded to a DM-flagged ticket (see
    /// `ticket::ChannelTicket::dm`) -- never for a bare name, since a
    /// brand-new channel created that way has no ticket to read a `dm`
    /// flag from (see `app::InputAction::StartDm` and `NetEvent::DmJoined`
    /// for how the creator's own side is handled instead).
    pub fn join(&self, arg: String) -> Option<String> {
        let (name, ticket) = match parse_join_arg(&arg) {
            Ok(parsed) => parsed,
            Err(error) => return Some(error),
        };
        if name.is_empty() {
            return None;
        }
        if name == DEVICE_SYNC_CHANNEL {
            return Some(format!(
                "#{DEVICE_SYNC_CHANNEL} is reserved for syncing your own paired devices and can't be joined directly"
            ));
        }

        let existing_secret = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .get(&name)
            .map(|c| c.secret);
        let ticket_secret = ticket.as_ref().map(|(secret, _, _)| *secret);
        if let (Some(existing), Some(from_ticket)) = (existing_secret, ticket_secret)
            && existing != from_ticket
        {
            return Some(format!(
                "that ticket is for a different channel also named #{name} -- pick a new local name to join it"
            ));
        }

        let self_ticket = ticket
            .as_ref()
            .is_some_and(|(_, addr, _)| is_self(addr, self.our_id));
        // The peer a DM-flagged ticket names as its sharer, so this side
        // can record them as our DM partner for `name` too (see
        // `crate::dm_registry`) -- `None` for a bare name (nothing to read
        // a `dm` flag from), an ordinary ticket (`dm: false`), or a
        // self-referential one (can't DM yourself).
        let dm_peer = ticket
            .as_ref()
            .filter(|(_, _, dm)| *dm)
            .filter(|_| !self_ticket)
            .map(|(_, addr, _)| *addr.id.as_bytes());

        if existing_secret.is_some() {
            let event = match dm_peer {
                Some(peer) => NetEvent::DmJoined { channel: name, peer },
                None => NetEvent::Joined(name),
            };
            let _ = self.events_tx.try_send(event);
            return None;
        }
        let secret = ticket_secret.unwrap_or_else(RoomSecret::generate);
        let bootstrap = ticket.map(|(_, addr, _)| addr);
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
                    let event = match dm_peer {
                        Some(peer) => NetEvent::DmJoined { channel: name, peer },
                        None => NetEvent::Joined(name),
                    };
                    let _ = events_tx.send(event).await;
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

    /// Joins `name` using an already-known `secret` and `bootstrap` peer
    /// addresses learned from a `crate::message::ChannelSyncAnnounce`
    /// (`sync_channel`), rather than typed by the user or carried in a
    /// ticket. Mirrors `join`'s spawn-then-report shape for a brand-new
    /// channel, but skips ticket parsing, self-reference messaging, and
    /// DM bookkeeping entirely, since none of that applies here -- just
    /// filters out any bootstrap address naming our own id, the same
    /// reason `join` does. Reports success as `NetEvent::ChannelSyncJoined`
    /// rather than `Joined`, so the caller can add a background tab
    /// without switching to it (see that variant's doc comment). A no-op
    /// if `name` is already joined, matching `join`'s own idempotent
    /// re-join behavior.
    pub fn join_known(&self, name: String, secret: RoomSecret, bootstrap: Vec<EndpointAddr>) {
        if self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .contains_key(&name)
        {
            return;
        }
        let bootstrap: Vec<EndpointAddr> = bootstrap
            .into_iter()
            .filter(|addr| !is_self(addr, self.our_id))
            .collect();

        let topic = topic_for_secret(&secret);
        let gossip = self.gossip.clone();
        let address_lookup = self.address_lookup.clone();
        let channels = Arc::clone(&self.channels);
        let events_tx = self.events_tx.clone();
        let endpoint = self.router.endpoint().clone();
        tokio::spawn(async move {
            for addr in &bootstrap {
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
                    let _ = events_tx.send(NetEvent::ChannelSyncJoined(name)).await;
                }
                Err(err) => {
                    let _ = events_tx
                        .send(NetEvent::JoinFailed(name, err.to_string()))
                        .await;
                }
            }
        });
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

    /// Announces our build version to `channel`'s direct gossip neighbors --
    /// called whenever a channel gains one, the same way `announce_nickname`
    /// re-sends the current nickname (see net.rs's handling of
    /// `iroh_gossip`'s `NeighborUp`). Unlike a nickname, our version is
    /// always known (never unset), so there's no early-return "nothing to
    /// announce" case here. Fire-and-forget, like `announce_nickname`.
    pub fn announce_version(&self, channel: &str) {
        let Some(sender) = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .get(channel)
            .map(|c| c.sender.clone())
        else {
            return;
        };
        let payload = GossipPayload::Version(VersionAnnounce {
            sender: self.our_id,
            version: version::VERSION.to_string(),
            git_hash: version::GIT_HASH.to_string(),
        });
        tokio::spawn(async move {
            let bytes = match postcard::to_stdvec(&payload) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("failed to encode version announce: {err}");
                    return;
                }
            };
            if let Err(err) = sender.broadcast_neighbors(bytes.into()).await {
                warn!("failed to broadcast version announce: {err}");
            }
        });
    }

    /// Announces (over `DEVICE_SYNC_CHANNEL` only) that we just joined
    /// `name`, so sibling devices learn about it too -- see
    /// `crate::message::ChannelSyncAnnounce` and `sync_channel`. Includes
    /// our own address as a bootstrap peer, so a sibling has an immediate,
    /// concrete way to reach the new channel's mesh instead of only a bare
    /// topic id. Uses a full-mesh `broadcast`, like `announce`/`send` --
    /// this is a one-shot event, never re-sent on a later `NeighborUp` the
    /// way `announce_nickname`/`announce_version`/`announce_device` are,
    /// so it needs the stronger delivery guarantee to reach every current
    /// member reliably. A no-op if `name` or `DEVICE_SYNC_CHANNEL` itself
    /// isn't actually joined (both should be impossible by the time this
    /// is called, but this avoids a panic if that assumption is ever
    /// broken).
    pub fn announce_channel_joined(&self, name: &str) {
        let Some(secret) = self.secret_for(name) else {
            return;
        };
        let Some(sender) = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .get(DEVICE_SYNC_CHANNEL)
            .map(|c| c.sender.clone())
        else {
            return;
        };
        let payload = GossipPayload::ChannelJoined(ChannelSyncAnnounce {
            sender: self.our_id,
            name: name.to_string(),
            secret,
            peers: vec![self.router.endpoint().addr()],
        });
        tokio::spawn(async move {
            let bytes = match postcard::to_stdvec(&payload) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("failed to encode channel-sync announce: {err}");
                    return;
                }
            };
            if let Err(err) = sender.broadcast(bytes.into()).await {
                warn!("failed to broadcast channel-sync announce: {err}");
            }
        });
    }

    /// Announces our device certificate (see `crate::message::DeviceCert`)
    /// to `channel`'s direct gossip neighbors -- called whenever a channel
    /// gains one, the same way `announce_nickname`/`announce_version` do.
    /// Every device always has `our_cert` from the moment `Net::start`
    /// finishes, so -- like `announce_version` -- there's no "nothing to
    /// announce yet" case here. Fire-and-forget, like `announce_version`.
    pub fn announce_device(&self, channel: &str) {
        let Some(sender) = self
            .channels
            .lock()
            .expect("channels lock poisoned")
            .get(channel)
            .map(|c| c.sender.clone())
        else {
            return;
        };
        let payload = GossipPayload::Device(self.our_cert.clone());
        tokio::spawn(async move {
            let bytes = match postcard::to_stdvec(&payload) {
                Ok(bytes) => bytes,
                Err(err) => {
                    warn!("failed to encode device cert announce: {err}");
                    return;
                }
            };
            if let Err(err) = sender.broadcast_neighbors(bytes.into()).await {
                warn!("failed to broadcast device cert announce: {err}");
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

    /// Reacts to a received `ChannelSyncAnnounce` (see `NetEvent::
    /// ChannelSync`): if it actually arrived on `DEVICE_SYNC_CHANNEL` and
    /// isn't an echo of our own announce, joins the named channel using
    /// its already-known secret and peer addresses (`join_known`) rather
    /// than generating a fresh one or needing a ticket. Ignores one that
    /// arrived on any other channel -- see `DEVICE_SYNC_CHANNEL`'s doc
    /// comment for why that check matters. Fire-and-forget, like
    /// `sync_history`.
    pub fn sync_channel(&self, channel: &str, announce: ChannelSyncAnnounce) {
        if channel != DEVICE_SYNC_CHANNEL || announce.sender == self.our_id {
            return;
        }
        self.join_known(announce.name, announce.secret, announce.peers);
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
        let path = expand_path(&path);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(err) => return Some(format!("can't send {}: {err}", path.display())),
        };
        if !metadata.is_file() {
            return Some(format!("can't send {}: not a file", path.display()));
        }
        if metadata.len() > MAX_FILE_SIZE {
            return Some(format!(
                "can't send {}: too large ({}, max {})",
                path.display(),
                crate::files::human_size(metadata.len()),
                crate::files::human_size(MAX_FILE_SIZE)
            ));
        }
        let Some(filename) = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_string)
        else {
            return Some(format!(
                "can't send {}: can't determine a filename",
                path.display()
            ));
        };

        let backfill = self.backfill.clone();
        let events_tx = self.events_tx.clone();
        let our_id = self.our_id;
        let size = metadata.len();
        tokio::spawn(async move {
            let hash = match backfill.add_file(&path).await {
                Ok(hash) => hash,
                Err(err) => {
                    let _ = events_tx
                        .send(NetEvent::FileSendFailed {
                            path: path.display().to_string(),
                            error: err.to_string(),
                        })
                        .await;
                    return;
                }
            };
            let message = ChatMessage {
                v: 3,
                id: rand::random(),
                sender: our_id,
                ts_unix_ms: now_unix_ms(),
                text: String::new(),
                attachment: Some(FileAttachment {
                    filename,
                    size,
                    hash,
                }),
                reply_to: None,
            };
            let _ = events_tx.send(NetEvent::FileReady(channel, message)).await;
        });

        None
    }

    /// Reads the OS clipboard and reports back what it found -- for
    /// `/paste` (or the `Ctrl+V`/`Cmd+V` keybinding), see
    /// `app::InputAction::Paste`. Fire-and-forget, like `announce` and
    /// `save_file`: unlike `send_file`, there's no cheap synchronous check
    /// to do first (we don't know what's on the clipboard until we ask),
    /// so the whole thing runs in a spawned task, reporting exactly one of
    /// `NetEvent::ClipboardFiles`/`ClipboardImage`/`ClipboardText`/
    /// `PasteFailed`.
    ///
    /// Opens a brand-new `Clipboard` rather than reusing main.rs's
    /// long-lived one (which stays dedicated to `/invite`'s writes): a
    /// clipboard read can block on a slow clipboard-owner round-trip
    /// (notably on X11), and arboard explicitly supports any number of
    /// `Clipboard` instances existing at once, so there's no need to share
    /// one just to read from it here.
    ///
    /// Tries, in priority order: a real file list (so a Finder/Explorer
    /// copy -- including an actual animated GIF -- shares its exact
    /// original bytes via the same path `/send` uses), then rendered image
    /// pixels (always a single static frame once re-encoded, see
    /// `encode_png`), then plain text (shared as an ordinary chat message).
    pub fn paste(&self, channel: String) {
        let events_tx = self.events_tx.clone();
        tokio::spawn(async move {
            let mut clipboard = match Clipboard::new() {
                Ok(clipboard) => clipboard,
                Err(err) => {
                    let _ = events_tx
                        .send(NetEvent::PasteFailed(format!(
                            "clipboard unavailable: {err}"
                        )))
                        .await;
                    return;
                }
            };

            if let Ok(paths) = clipboard.get().file_list()
                && !paths.is_empty()
            {
                let _ = events_tx
                    .send(NetEvent::ClipboardFiles(channel, paths))
                    .await;
                return;
            }

            if let Ok(image) = clipboard.get_image() {
                match encode_png(image) {
                    Ok(bytes) => {
                        let filename = format!("clipboard-{}.png", now_unix_ms());
                        let _ = events_tx
                            .send(NetEvent::ClipboardImage(channel, filename, bytes))
                            .await;
                    }
                    Err(err) => {
                        let _ = events_tx
                            .send(NetEvent::PasteFailed(format!(
                                "failed to encode clipboard image: {err}"
                            )))
                            .await;
                    }
                }
                return;
            }

            if let Ok(text) = clipboard.get_text()
                && !text.is_empty()
            {
                let _ = events_tx.send(NetEvent::ClipboardText(channel, text)).await;
                return;
            }

            let _ = events_tx
                .send(NetEvent::PasteFailed("clipboard is empty".to_string()))
                .await;
        });
    }

    /// Imports already-in-hand bytes (a clipboard image re-encoded as PNG
    /// by `paste`/`encode_png`) into the shared blob store and prepares
    /// them to broadcast to `channel` as a `ChatMessage` with an
    /// attachment -- mirrors `send_file`, minus the path-stat step, since
    /// there's no real path to check; `bytes.len()` is checked against the
    /// same `MAX_FILE_SIZE` cap instead.
    pub fn send_clipboard_image(
        &self,
        channel: String,
        filename: String,
        bytes: Vec<u8>,
    ) -> Option<String> {
        let size = bytes.len() as u64;
        if size > MAX_FILE_SIZE {
            return Some(format!(
                "can't paste image: too large ({}, max {})",
                crate::files::human_size(size),
                crate::files::human_size(MAX_FILE_SIZE)
            ));
        }

        let backfill = self.backfill.clone();
        let events_tx = self.events_tx.clone();
        let our_id = self.our_id;
        tokio::spawn(async move {
            let hash = match backfill.add_bytes(bytes).await {
                Ok(hash) => hash,
                Err(err) => {
                    let _ = events_tx
                        .send(NetEvent::PasteFailed(format!(
                            "failed to import clipboard image: {err}"
                        )))
                        .await;
                    return;
                }
            };
            let message = ChatMessage {
                v: 3,
                id: rand::random(),
                sender: our_id,
                ts_unix_ms: now_unix_ms(),
                text: String::new(),
                attachment: Some(FileAttachment {
                    filename,
                    size,
                    hash,
                }),
                reply_to: None,
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
                    // `destination` was already claimed as an empty
                    // placeholder by `files::resolve_destination` before
                    // this task started (so concurrent saves can't race
                    // onto the same name) -- clean it up on failure so a
                    // failed save doesn't leave a stray empty file behind.
                    let _ = std::fs::remove_file(&destination);
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

    /// Creates a one-time pairing ticket (see `pairing::PairingProtocol`
    /// and `ticket::PairingTicket`) for `/pair` to show: this device's own
    /// address, the shared user key, and `channels` -- a snapshot of every
    /// currently-known channel, in the same shape `Net::start` accepts as
    /// `known_channels` -- so a brand-new device that redeems it can seed
    /// its own channel registry and join everything through the normal
    /// startup path, exactly as if it had been running all along. Our own
    /// address is appended to every channel's bootstrap list -- including
    /// one with no previously-learned peers at all, the common case for a
    /// channel nobody else has ever joined -- mirroring
    /// `announce_channel_joined`'s same reasoning: without a concrete
    /// address to dial, the redeeming device would have nothing but a bare
    /// topic id for each channel and could never actually form a gossip
    /// mesh with the very device it just paired with. Synchronous, like
    /// `ticket_for`.
    pub fn create_pairing_ticket(
        &self,
        channels: Vec<(String, RoomSecret, Vec<EndpointAddr>)>,
    ) -> PairingTicket {
        let our_addr = self.router.endpoint().addr();
        let channels = channels
            .into_iter()
            .map(|(name, secret, mut peers)| {
                peers.push(our_addr.clone());
                (name, secret, peers)
            })
            .collect();
        self.pairing
            .create_ticket(our_addr, self.user_key_bytes, channels)
    }

    /// Builds this instance's invite ticket string for `channel`: our
    /// current address, the channel's room secret, and its display name,
    /// ready to be pasted into another instance's `--join` flag or `/join`
    /// command. `dm` marks it as a 1:1 DM invite rather than an ordinary
    /// group one (see `ticket::ChannelTicket::dm` and `NetEvent::DmJoined`)
    /// -- callers decide this by checking `crate::dm_registry` themselves,
    /// since `Net` has no notion of a channel's "kind" of its own.
    /// Synchronous -- `Endpoint::addr` doesn't need to await anything.
    ///
    /// # Panics
    /// Panics if `channel` isn't currently joined -- every call site
    /// (startup's own just-joined list, and `/invite`'s active channel)
    /// only ever names a channel we're already in.
    pub fn ticket_for(&self, channel: &str, dm: bool) -> String {
        let secret = self
            .secret_for(channel)
            .expect("ticket_for called for an unjoined channel");
        ChannelTicket {
            name: channel.to_string(),
            secret,
            addr: self.router.endpoint().addr(),
            dm,
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
                    Ok(GossipPayload::Version(version)) => {
                        NetEvent::Version(channel.clone(), version)
                    }
                    Ok(GossipPayload::Device(cert)) if cert.is_valid() => {
                        NetEvent::Device(channel.clone(), cert)
                    }
                    Ok(GossipPayload::Device(_)) => {
                        warn!("dropping device certificate with an invalid signature");
                        continue;
                    }
                    Ok(GossipPayload::ChannelJoined(announce)) => {
                        NetEvent::ChannelSync(channel.clone(), announce)
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
            dm: false,
        }
        .encode_string();

        let (name, ticket) = parse_join_arg(&encoded).unwrap();
        assert_eq!(name, "general");
        assert_eq!(ticket, Some((secret, addr, false)));
    }

    #[test]
    fn parse_join_arg_carries_the_dm_flag_through() {
        let addr = sample_addr();
        let secret = RoomSecret::generate();
        let encoded = ChannelTicket {
            name: "dm-alice".to_string(),
            secret,
            addr: addr.clone(),
            dm: true,
        }
        .encode_string();

        let (name, ticket) = parse_join_arg(&encoded).unwrap();
        assert_eq!(name, "dm-alice");
        assert_eq!(ticket, Some((secret, addr, true)));
    }

    #[test]
    fn parse_join_arg_reports_a_truncated_ticket_as_an_error_not_a_name() {
        let ticket = ChannelTicket {
            name: "general".to_string(),
            secret: RoomSecret::generate(),
            addr: sample_addr(),
            dm: false,
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

    fn home_dir() -> PathBuf {
        directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf()
    }

    #[test]
    fn expand_path_expands_a_bare_tilde_to_the_home_directory() {
        assert_eq!(expand_path("~"), home_dir());
    }

    #[test]
    fn expand_path_expands_a_tilde_prefixed_path() {
        assert_eq!(
            expand_path("~/Downloads/tilemap.png"),
            home_dir().join("Downloads/tilemap.png")
        );
    }

    #[test]
    fn expand_path_makes_a_relative_path_absolute() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(expand_path("README.md"), cwd.join("README.md"));
    }

    #[test]
    fn expand_path_leaves_an_already_absolute_path_alone() {
        assert_eq!(expand_path("/tmp/foo.txt"), PathBuf::from("/tmp/foo.txt"));
    }

    #[test]
    fn expand_path_does_not_expand_a_tilde_in_the_middle_of_a_path() {
        // Only a leading `~` is special, mirroring shell tilde expansion --
        // `foo/~/bar` is just a literal (if unusual) relative path.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(expand_path("foo/~/bar"), cwd.join("foo/~/bar"));
    }

    #[test]
    fn encode_png_round_trips_known_pixel_data() {
        // A 2x1 image: one red, one green pixel (RGBA8, row-major -- see
        // `ImageData`'s doc comment).
        let image = ImageData {
            width: 2,
            height: 1,
            bytes: std::borrow::Cow::Owned(vec![255, 0, 0, 255, 0, 255, 0, 255]),
        };

        let png_bytes = encode_png(image).unwrap();

        let decoded = image::load_from_memory_with_format(&png_bytes, ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 1));
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(decoded.get_pixel(1, 0).0, [0, 255, 0, 255]);
    }

    #[test]
    fn encode_png_rejects_a_buffer_that_does_not_match_its_dimensions() {
        let image = ImageData {
            width: 4,
            height: 4,
            bytes: std::borrow::Cow::Owned(vec![0; 4]), // far too few bytes for 4x4 RGBA8
        };

        assert!(encode_png(image).is_err());
    }
}
