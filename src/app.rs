//! Owns all UI state and the pure logic that mutates it.
//!
//! Per concept.md's "one owner of UI state" principle: nothing here does
//! network or file I/O. `handle_key` is a pure function of the current
//! state and a keypress -- when the user sends a message or runs a `/join`
//! or `/invite` command, it returns an `InputAction` describing what the
//! caller should do, rather than reaching for I/O itself.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use iroh_blobs::Hash;
use iroh_tickets::Ticket;

use crate::autosave::AutoSaveLimit;
#[cfg(test)]
use crate::autosave::DEFAULT_AUTOSAVE_LIMIT;
use crate::files::{
    DEFAULT_MAX_FILE_SIZE, SaveMode, SizeLimit, StagedAttachment, channel_dir, human_size,
    runs_when_opened,
};
use crate::message::{ChatMessage, now_unix_ms};
use crate::net::{DEVICE_SYNC_CHANNEL, NetEvent};
use crate::picker::{FilePicker, FolderListing, Step};
use crate::search::{self, SearchOutcome, SearchResults};
pub use crate::shared_files::SharedFile;
use crate::sync_state::{SyncConfig, SyncPolicy, SyncRule};
use crate::thread::{self, ThreadView};
use crate::ticket::PairingTicket;
use crate::version;

mod files_overlay;
mod net_events;
mod replies;

/// A peer shown in the active channel's sidebar. `last_seen_unix_ms` is
/// absent only while the peer is currently online.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerPresence {
    pub id: [u8; 32],
    pub last_seen_unix_ms: Option<u64>,
}

/// Identity and presence details for one row in a peer presentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerDetails {
    /// The hovered device's id, for its identity colour.
    pub id: [u8; 32],
    /// Our local `/alias` for them, if any.
    pub alias: Option<String>,
    /// Their self-chosen `/nick`, if they've announced one.
    pub nickname: Option<String>,
    /// `None` while online, as in `PeerPresence`.
    pub last_seen_unix_ms: Option<u64>,
    /// Their announced leyline build version, if heard.
    pub version: Option<String>,
}

/// How long a valid full-channel device certificate keeps a non-neighbor
/// device online. The runtime refreshes these observations every
/// `HISTORY_ANNOUNCE_INTERVAL`, so three missed announcements are tolerated
/// before the sidebar falls back to the persisted offline row.
const PEER_ACTIVITY_TTL_MS: u64 = 90_000;

/// Cap on in-memory scrollback per channel, per concept.md's guidance on
/// bounding render/memory cost in a long-lived session.
const MAX_SCROLLBACK: usize = 500;

/// One user-facing slash command. This catalog is the shared source for the
/// command picker and sidebar hints; `input` is the canonical compose text
/// when the picker inserts the command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandDefinition {
    pub syntax: &'static str,
    pub input: &'static str,
    pub picker_action: CommandPickerAction,
    /// A global keybinding for this command or its more specific picker, if
    /// one exists.
    pub shortcut: Option<&'static str>,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
}
/// What selecting a command from Ctrl+K does. This is explicit catalog
/// metadata rather than an inference from optional or required syntax: some
/// optional commands have useful safe defaults, while others need deliberate
/// user input before their bare behavior should be invoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPickerAction {
    Execute,
    Insert,
}

const COMMANDS: &[CommandDefinition] = &[
    CommandDefinition {
        syntax: "/join <name|ticket>",
        input: "/join ",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Join or create a channel",
    },
    CommandDefinition {
        syntax: "/invite",
        input: "/invite",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Show an invite for this channel",
    },
    CommandDefinition {
        syntax: "/pair [ticket]",
        input: "/pair ",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Pair a device or show a pairing ticket",
    },
    CommandDefinition {
        syntax: "/forget-device <hex-prefix>",
        input: "/forget-device ",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Forget a paired device",
    },
    CommandDefinition {
        syntax: "/msg <peer> [text]",
        input: "/msg ",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Start a private channel message",
    },
    CommandDefinition {
        syntax: "/dm <peer> [text]",
        input: "/dm ",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Start a direct peer conversation",
    },
    CommandDefinition {
        syntax: "/alias <hex-prefix> <name>",
        input: "/alias ",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Set a local name for a peer",
    },
    CommandDefinition {
        syntax: "/nick <name>",
        input: "/nick ",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Set your broadcast display name",
    },
    CommandDefinition {
        syntax: "/leave [channel]",
        input: "/leave",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Leave a channel",
    },
    CommandDefinition {
        syntax: "/rename [name]",
        input: "/rename ",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Set this channel's local display name",
    },
    CommandDefinition {
        syntax: "/who",
        input: "/who",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "View this device and active peer details",
    },
    CommandDefinition {
        syntax: "/send [path]",
        input: "/send",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Pick or share a file",
    },
    CommandDefinition {
        syntax: "/save [hash-prefix]",
        input: "/save",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &[],
        description: "Download a shared file",
    },
    CommandDefinition {
        syntax: "/folder [path]",
        input: "/folder",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Show or set the files folder",
    },
    CommandDefinition {
        syntax: "/maxsize [size|off|default]",
        input: "/maxsize",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Show or set the file size limit",
    },
    CommandDefinition {
        syntax: "/autosave [on|off|limit]",
        input: "/autosave",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Configure automatic file saving",
    },
    CommandDefinition {
        syntax: "/outbox [on|off]",
        input: "/outbox",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Configure the watched send folder",
    },
    CommandDefinition {
        syntax: "/sync [options]",
        input: "/sync",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Configure folder synchronization",
    },
    CommandDefinition {
        syntax: "/activity",
        input: "/activity",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Show recent sync activity",
    },
    CommandDefinition {
        syntax: "/conflicts [resolution]",
        input: "/conflicts",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "List or resolve sync conflicts",
    },
    CommandDefinition {
        syntax: "/trash [restore|purge]",
        input: "/trash",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Manage recoverable sync deletes",
    },
    CommandDefinition {
        syntax: "/files",
        input: "/files",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Browse shared and synced channel files",
    },
    CommandDefinition {
        syntax: "/paste",
        input: "/paste",
        picker_action: CommandPickerAction::Execute,
        shortcut: Some("Ctrl+V"),
        aliases: &[],
        description: "Share the clipboard",
    },
    CommandDefinition {
        syntax: "/search <term>",
        input: "/search ",
        picker_action: CommandPickerAction::Insert,
        shortcut: None,
        aliases: &["/s"],
        description: "Search channel messages",
    },
    CommandDefinition {
        syntax: "/reply [text]",
        input: "/reply",
        picker_action: CommandPickerAction::Execute,
        shortcut: Some("Ctrl+R"),
        aliases: &[],
        description: "Reply to the latest message",
    },
    CommandDefinition {
        syntax: "/thread",
        input: "/thread",
        picker_action: CommandPickerAction::Execute,
        shortcut: Some("Ctrl+T"),
        aliases: &[],
        description: "View the latest reply thread",
    },
    CommandDefinition {
        syntax: "/hints",
        input: "/hints",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Toggle the command hints panel",
    },
    CommandDefinition {
        syntax: "/mouse",
        input: "/mouse",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Toggle mouse hover popups",
    },
    CommandDefinition {
        syntax: "/bell",
        input: "/bell",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Toggle the message bell",
    },
    CommandDefinition {
        syntax: "/version",
        input: "/version",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Show this build's version",
    },
    CommandDefinition {
        syntax: "/help",
        input: "/help",
        picker_action: CommandPickerAction::Execute,
        shortcut: None,
        aliases: &[],
        description: "Show command and keybinding help",
    },
];

pub fn command_catalog() -> &'static [CommandDefinition] {
    COMMANDS
}

/// A line in a channel's transcript: either a real chat message or a
/// local-only notice (invite output, command errors). System lines never
/// travel over the network -- see message.rs for the wire format. Named
/// `TranscriptLine` (not `Line`) to avoid colliding with `ratatui::text::Line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptLine {
    Chat(ChatMessage),
    System(String),
}

/// Runtime observations for one channel's sync, kept in app state so the UI
/// and `/sync` report one pure snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncConflict {
    pub local_path: String,
    pub remote_path: String,
    pub remote_copy_path: Option<String>,
    pub local_modified_unix_ms: Option<u64>,
    pub remote_modified_unix_ms: Option<u64>,
    pub author: Option<[u8; 32]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    Local,
    Remote,
    Both,
}

#[derive(Debug, Default)]
pub struct SyncStatus {
    pending: HashSet<String>,
    last_success_unix_ms: Option<u64>,
    errors: Vec<(String, String)>,
    conflicts: Vec<SyncConflict>,
}

impl SyncStatus {
    fn pending(&mut self, path: String) {
        self.pending.insert(path);
    }

    fn succeeded(&mut self, path: &str, completed_unix_ms: u64) {
        self.pending.remove(path);
        self.errors.retain(|(failed, _)| failed != path);
        self.last_success_unix_ms = Some(completed_unix_ms);
    }

    fn failed(&mut self, path: String, error: String) {
        self.pending.remove(&path);
        self.errors.retain(|(failed, _)| failed != &path);
        self.errors.push((path, error));
    }

    fn conflict(&mut self, conflict: SyncConflict) {
        if !self.conflicts.iter().any(|known| {
            known.local_path == conflict.local_path && known.remote_path == conflict.remote_path
        }) {
            self.conflicts.push(conflict);
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    pub fn conflict_count(&self) -> usize {
        self.conflicts.len()
    }

    pub fn conflicts(&self) -> &[SyncConflict] {
        &self.conflicts
    }
}

/// What kind of conversation a channel is -- see `AppState::channel_kind`.
/// "Group" is simply the absence of an association with a single peer, so
/// every channel is exactly one of these, and the 1:1 kinds can't exist
/// without saying who they're with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelKind {
    /// An ordinary named channel: a gossip topic joined by name or ticket,
    /// shared with any number of peers.
    Group,
    /// A `/msg` DM: a private gossip channel provisioned for one peer (see
    /// `AppState::run_msg` and `crate::dm_registry`).
    GossipDm { peer: [u8; 32] },
    /// A `/dm` point-to-point conversation with one peer, with no gossip
    /// topic behind it (see `crate::direct`).
    Direct { peer: [u8; 32] },
}

impl ChannelKind {
    /// The one-symbol badge that tells this kind apart at a glance -- shown
    /// beside every channel in the navigation panel and in front of its
    /// name in titles and notices (`AppState::channel_heading`). The
    /// point-to-point badge is double-width in a terminal, which is why
    /// `ui::render_nav` pads every badge to a fixed column instead of
    /// assuming one cell.
    pub fn glyph(self) -> &'static str {
        match self {
            ChannelKind::Group => "#",
            ChannelKind::GossipDm { .. } => "@",
            ChannelKind::Direct { .. } => "⚡",
        }
    }
}

/// Which channels the navigation panel lists under each of its two
/// sections, as indices into `AppState::channels` -- see
/// `AppState::nav_sections`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavSections {
    /// Group channels, in join order.
    pub channels: Vec<usize>,
    /// `/msg` DMs and `/dm` point-to-point chats, in join order.
    pub people: Vec<usize>,
}

/// The jump-to-conversation overlay opened with `Ctrl+P` -- see
/// `AppState::switching`. Only the query and the highlight are state:
/// which channels match is always derived from the live channel list
/// (`AppState::switcher_matches`), so the list itself can never go stale.
#[derive(Debug, Default)]
pub struct Switcher {
    /// What's been typed so far, matched case-insensitively against each
    /// channel's label.
    pub query: String,
    /// Position of the highlighted entry among the current matches. Reset
    /// to the first match whenever the query changes
    /// (`AppState::edit_switcher_query`) and clamped on every read
    /// (`AppState::switcher_highlight`), since the matches can change under
    /// an open overlay -- a peer gets renamed, a channel joins.
    selected: usize,
}

/// The slash-command picker opened with `Ctrl+K`. Entries come from
/// `command_catalog`, so only its query and highlighted position are state.
#[derive(Debug, Default)]
pub struct CommandPicker {
    pub query: String,
    selected: usize,
}

/// The peer-details overlay opened with `/who`. It snapshots the active
/// channel's online and remembered peers plus their current identity details
/// so a later focus change or network update cannot change the table out from
/// under the user.
#[derive(Debug)]
pub struct PeersOverlay {
    pub heading: String,
    pub peers: Vec<PeerDetails>,
}

/// The files overlay opened with `/files` -- see `AppState::viewing_files`.
/// Like `Switcher` it stores no list: the files are always derived from the
/// channel's live transcript (`AppState::files_matches`), so the list can't
/// go stale, and the highlight is a position clamped on every read
/// (`AppState::files_highlight`). It keeps two things more, both because it
/// acts on one channel's files: which channel, and what the disk said about
/// them.
#[derive(Debug)]
pub struct FilesOverlay {
    /// The channel whose files it lists, by stored name, fixed when the
    /// overlay opened. Focus can move under an open overlay (a join that
    /// finishes switches to its channel, `activate_or_create_channel`), and
    /// `Enter` must save into the folder of the channel whose list the user
    /// was looking at.
    pub channel: String,
    /// Identity of the background folder scan requested when this overlay
    /// opened. A late response may only update this exact overlay.
    request: u64,
    /// What's been typed so far, matched case-insensitively against each
    /// file's name and its sender as shown (`AppState::display_name`).
    pub query: String,
    /// Position of the highlighted file among the current matches. Reset to
    /// the first match whenever the query changes
    /// (`AppState::edit_files_query`) and clamped on every read
    /// (`AppState::files_highlight`), since the matches can change under an
    /// open overlay -- a file arrives, a peer gets renamed.
    selected: usize,
    /// Where this channel's saved copy of each file is, for the files whose
    /// copy was on disk when the overlay opened (`InputAction::
    /// CheckSavedFiles`, answered through `AppState::apply_saved_files`) or
    /// has landed since (`NetEvent::FileSaved`). What the overlay marks as
    /// saved, and what `Ctrl+O` opens. It is the disk's word as of that
    /// moment -- app.rs can't look for itself -- and nothing outside an open
    /// overlay needs it, so it lives and dies with it.
    saved: HashMap<Hash, PathBuf>,
    /// Eligible regular files below this channel's folder, supplied by the
    /// runtime after its background scan.
    local: Vec<LocalFile>,
    /// What the last key did not do, and why -- see `FilesNotice`. Dropped by
    /// the next key (`AppState::handle_files_key`) or paste
    /// (`AppState::edit_files_query`).
    pub notice: Option<FilesNotice>,
}

/// Why a key in the files overlay did nothing, said in its bottom border
/// until the next key (`FilesOverlay::notice`). `Enter` and `Ctrl+O` each fit
/// only some of the files (`FileOffer`), and a key that doesn't fit the
/// highlighted one must not look like a dead key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesNotice {
    /// `Enter` on a file that is already saved and can be opened: it is not
    /// saved a second time, `Ctrl+O` opens it.
    AlreadySaved,
    /// `Enter` or `Ctrl+O` on a file that is already saved but runs
    /// something when opened (`files::runs_when_opened`): it is not saved a
    /// second time, and leyline won't open it -- a program or script is
    /// opened by hand, from the file manager, if at all.
    SavedNotOpened,
    /// An operation requires a local copy, but this attachment is not saved.
    NotLocal,
    /// Enter saves remote attachments; a sync-local file is already present.
    LocalFile,
}

/// What `Enter` and `Ctrl+O` can do for a file in the files overlay, worked
/// out in one place (`AppState::file_offer`) so the bottom border and the
/// two keys can't disagree about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOffer {
    /// Not saved: `Enter` saves it.
    Save,
    /// Saved, and the copy at this path can be opened: `Ctrl+O` opens it.
    Open(PathBuf),
    /// Saved, but a kind of file that runs something when opened
    /// (`files::runs_when_opened`): neither key has anything to do for it.
    SavedOnly,
}

/// One safe regular file discovered below a channel's local folder. The
/// runtime owns the filesystem scan; app state only stores the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFile {
    pub path: PathBuf,
    pub relative: String,
    pub size: u64,
    pub modified_unix_ms: u64,
}

/// One row in the `/files` overlay: either an attachment that can be saved
/// from peers or a regular file already present in the channel folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesEntry {
    Shared(SharedFile),
    Local(LocalFile),
}

/// Result of one asynchronous `/files` folder scan.
pub struct FilesListing {
    pub request: u64,
    pub channel: String,
    pub files: Result<Vec<LocalFile>, String>,
}

/// One joined channel's transcript, presence, per-channel scroll state,
/// and in-progress input draft.
pub struct Channel {
    pub name: String,
    pub messages: VecDeque<TranscriptLine>,
    /// Direct iroh-gossip neighbors, updated immediately by
    /// `NeighborUp`/`NeighborDown`.
    pub peers: Vec<[u8; 32]>,
    /// Valid device certificates received from elsewhere in the gossip mesh,
    /// keyed by the certified endpoint id and held only until their bounded
    /// activity window expires. This is deliberately transient: `last_seen`
    /// supplies the durable offline history instead.
    recent_peers: HashMap<[u8; 32], u64>,
    last_seen: HashMap<[u8; 32], u64>,
    pub scroll: usize,
    /// Number of newly received live messages not yet viewed. This is
    /// transient, like `scroll`: startup history and backfill establish the
    /// transcript but never produce an unread badge.
    pub unread_count: usize,
    /// The oldest message contributing to `unread_count`, so activation can
    /// show it first even though wrapped-row layout belongs to `ui`.
    pub first_unread: Option<u64>,
    /// The first unread message to show after this channel becomes active.
    /// `ui` resolves it against the current terminal-width-specific rows;
    /// an explicit scroll clears it.
    pub unread_anchor: Option<u64>,
    /// Every message id seen so far in this channel (live gossip or
    /// backfill), for dedupe. Deliberately unbounded -- capping this by
    /// recency used to be the cause of a real duplication bug:
    /// `net::Net::sync_history` redelivers a channel's *entire* known
    /// history (`backfill::BackfillStore::fetch`) on every root mismatch,
    /// not just messages sent since the last sync, and in an active
    /// multi-peer channel roots mismatch essentially continuously. A
    /// bounded recency window let old-but-still-displayed message ids age
    /// out, so the next resync treated them as brand new -- redisplaying
    /// and re-persisting them -- and that repeated on every later resync,
    /// compounding indefinitely. A few bytes per message ever seen in a
    /// channel is a trivial memory cost next to keeping that channel's
    /// full history on disk anyway.
    seen_ids: HashSet<u64>,
    /// Active `/search <term>` results, if any -- see `AppState::run_search`
    /// and `ui::render_messages`. Mutually exclusive with `thread` (opening
    /// one clears the other): a channel shows at most one alternate view at
    /// a time. Transient UI state, like `scroll`: never persisted, and not
    /// kept live as new messages arrive (re-run `/search <term>` to
    /// refresh).
    pub search: Option<SearchResults>,
    /// Active `/thread` view, if any -- see `AppState::run_thread`/
    /// `open_thread` and `ui::render_messages`. Mutually exclusive with
    /// `search`, like above. Transient UI state, like `search`: never
    /// persisted. Unlike `search`, grows live as directly-connected
    /// messages are sent or arrive (`push`, via `ThreadView::try_append`)
    /// -- see that type's doc comment for the one case it still can't
    /// catch. A plain message sent while this is active automatically
    /// replies within it -- see `AppState::active_thread_reply_target`.
    pub thread: Option<ThreadView>,
    /// This channel's own composed-but-unsent message text -- kept per
    /// channel (not on `AppState`) so switching tabs
    /// (`AppState::switch_channel`) never loses a half-typed draft in one
    /// channel or leaks it into another's `Enter` (features.md's
    /// "Per-channel input drafts"). In-memory only, like `scroll`. The
    /// editing methods below (`insert_char`, `delete_backward`, etc.) only
    /// ever touch this and `cursor`, so they live on `Channel` rather than
    /// `AppState`.
    pub input: String,
    /// Character index (not byte offset -- see `byte_index`) of the
    /// cursor within `input`.
    pub cursor: usize,
    /// Files and images chosen to share (`Ctrl+V`/`/paste`) but not yet
    /// sent, shown in a strip above the input box (`ui::render_staged`)
    /// until `Enter` sends them all (`AppState::send_staged`), `Backspace`
    /// on an empty input takes the newest back out, or `Esc` clears the
    /// lot. Kept per channel, like `input`, so switching tabs
    /// (`AppState::switch_channel`) neither loses them nor leaks them into
    /// another channel's `Enter`. In-memory only, like `input`, and
    /// dropped with the channel.
    pub staged: Vec<StagedAttachment>,
}

impl Channel {
    fn new(name: String) -> Self {
        Self {
            name,
            messages: VecDeque::new(),
            peers: Vec::new(),
            recent_peers: HashMap::new(),
            last_seen: HashMap::new(),
            scroll: 0,
            unread_count: 0,
            first_unread: None,
            unread_anchor: None,
            seen_ids: HashSet::new(),
            search: None,
            thread: None,
            input: String::new(),
            cursor: 0,
            staged: Vec::new(),
        }
    }

    /// Records `id` as seen. Returns `true` if `id` had not already been
    /// seen (i.e. it should be displayed) -- see `seen_ids`'s doc comment
    /// for why this must never forget an id once recorded.
    fn remember_seen(&mut self, id: u64) -> bool {
        self.seen_ids.insert(id)
    }

    /// Appends `line` to this channel's transcript, enforcing
    /// `MAX_SCROLLBACK`. A new chat message is also offered to an active
    /// `/thread` view (`ThreadView::try_append`), so a reply made or
    /// received while reading a thread shows up immediately instead of
    /// only on the next `/thread`/`Ctrl+T` rebuild.
    fn push(&mut self, line: TranscriptLine) {
        if let TranscriptLine::Chat(message) = &line
            && let Some(thread) = &mut self.thread
        {
            thread.try_append(message);
        }
        self.messages.push_back(line);
        while self.messages.len() > MAX_SCROLLBACK {
            self.messages.pop_front();
        }
    }

    /// Seeds this channel's transcript and dedupe state from persisted
    /// history (see storage.rs), oldest message first. Reuses the same
    /// dedupe/cap logic as live messages so loaded history behaves
    /// identically to messages received during this session.
    fn seed_history(&mut self, messages: Vec<ChatMessage>) {
        for message in messages {
            if self.remember_seen(message.id) {
                self.push(TranscriptLine::Chat(message));
            }
        }
    }

    /// Scans this channel's loaded transcript for `Chat` lines whose text
    /// matches `term_lower` (already lowercased), oldest first -- the
    /// synchronous half of `/search`. See `search::run_on_disk_scan` for
    /// the background half covering history older than what's loaded here.
    fn search_loaded(&self, term_lower: &str) -> Vec<ChatMessage> {
        self.messages
            .iter()
            .filter_map(|line| match line {
                TranscriptLine::Chat(message) if search::message_matches(message, term_lower) => {
                    Some(message.clone())
                }
                _ => None,
            })
            .collect()
    }

    /// Ids of chat messages currently visible in this channel, oldest
    /// first: its active `/thread` view if one is open, else its active
    /// `/search` results if one is running (mirroring what's actually on
    /// screen), else its full loaded transcript (skipping `System` lines,
    /// which aren't reply targets). Shared by reply-pick navigation
    /// (`AppState::start_reply_pick`/`move_reply_pick`/`run_reply`) and
    /// thread-pick navigation (`start_thread_pick`/`move_thread_pick`), so
    /// picking only ever lands on a message the user can currently see.
    fn visible_chat_ids(&self) -> Vec<u64> {
        if let Some(thread) = &self.thread {
            return thread.messages.iter().map(|message| message.id).collect();
        }
        if let Some(search) = &self.search {
            return search.messages.iter().map(|message| message.id).collect();
        }
        self.messages
            .iter()
            .filter_map(|line| match line {
                TranscriptLine::Chat(message) => Some(message.id),
                TranscriptLine::System(_) => None,
            })
            .collect()
    }

    /// Finds a chat message by id in this channel, checking its active
    /// thread view first, then its active search results (if any --
    /// `search.messages` can hold matches from `search::run_on_disk_scan`
    /// that are older than what `self.messages` keeps under
    /// `MAX_SCROLLBACK`), and then falling back to the full loaded
    /// transcript. The fallback matters even with a thread or search
    /// active: a reply's quoted parent (`ui::reply_preview_spans`) should
    /// still resolve when it simply isn't part of the current view, unlike
    /// `visible_chat_ids`, which deliberately only offers up messages the
    /// user can currently see to *pick* a reply or thread target from.
    /// Also used to describe what `/reply` just armed
    /// (`AppState::run_reply`) and to resolve a thread's origin
    /// (`AppState::open_thread`). `None` covers a dangling `reply_to` just
    /// as much as an outright-unknown id -- see `ChatMessage::reply_to`'s
    /// doc comment.
    pub fn find_message(&self, id: u64) -> Option<&ChatMessage> {
        if let Some(thread) = &self.thread
            && let Some(message) = thread.messages.iter().find(|message| message.id == id)
        {
            return Some(message);
        }
        if let Some(search) = &self.search
            && let Some(message) = search.messages.iter().find(|message| message.id == id)
        {
            return Some(message);
        }
        self.messages.iter().find_map(|line| match line {
            TranscriptLine::Chat(message) if message.id == id => Some(message),
            _ => None,
        })
    }

    fn char_count(&self) -> usize {
        self.input.chars().count()
    }

    /// Byte offset in `input` corresponding to a character index.
    ///
    /// Needed because `String` indexing/mutation is byte-based, but the
    /// cursor is tracked as a character index so editing works correctly
    /// with multi-byte UTF-8 input.
    fn byte_index(&self, char_idx: usize) -> usize {
        self.input
            .char_indices()
            .nth(char_idx)
            .map(|(b, _)| b)
            .unwrap_or(self.input.len())
    }

    fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.char_count());
    }

    /// Char-index bounds of the line containing the cursor: the nearest
    /// `\n` before/after it (exclusive of the newline itself), or the
    /// buffer's edges if there is none. Multi-line input redefines "line"
    /// for `Home`/`End`/`Ctrl+A`/`Ctrl+E`/`Ctrl+U` from "the whole
    /// buffer" to this -- matching standard multi-line editor behavior --
    /// while `Left`/`Right`/`Backspace`/`Delete`/`Ctrl+W` stay simple
    /// char-index operations that already cross a `\n` boundary like any
    /// other character (e.g. `Left` at column 0 lands at the end of the
    /// previous line). A no-op change for single-line input, since then
    /// there's only ever one line spanning the whole buffer.
    fn current_line_bounds(&self) -> (usize, usize) {
        let chars: Vec<char> = self.input.chars().collect();
        let start = chars[..self.cursor]
            .iter()
            .rposition(|&c| c == '\n')
            .map_or(0, |i| i + 1);
        let end = chars[self.cursor..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(chars.len(), |i| self.cursor + i);
        (start, end)
    }

    fn move_home(&mut self) {
        self.cursor = self.current_line_bounds().0;
    }

    fn move_end(&mut self) {
        self.cursor = self.current_line_bounds().1;
    }

    fn insert_char(&mut self, c: char) {
        let idx = self.byte_index(self.cursor);
        self.input.insert(idx, c);
        self.cursor += 1;
    }

    /// Inserts a literal newline at the cursor without submitting --
    /// bound to `Alt+Enter`/`Ctrl+J` in `AppState::handle_key`, and used
    /// by `AppState::paste_text` for an embedded line break in pasted
    /// text.
    fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Backspace: deletes the character before the cursor.
    fn delete_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let end = self.byte_index(self.cursor);
        let start = self.byte_index(self.cursor - 1);
        self.input.replace_range(start..end, "");
        self.cursor -= 1;
    }

    /// Delete: deletes the character at the cursor.
    fn delete_forward(&mut self) {
        if self.cursor >= self.char_count() {
            return;
        }
        let start = self.byte_index(self.cursor);
        let end = self.byte_index(self.cursor + 1);
        self.input.replace_range(start..end, "");
    }

    /// Ctrl+U: deletes from the start of the current line to the cursor.
    fn delete_to_start(&mut self) {
        let (line_start, _) = self.current_line_bounds();
        let start = self.byte_index(line_start);
        let end = self.byte_index(self.cursor);
        self.input.replace_range(start..end, "");
        self.cursor = line_start;
    }

    /// Ctrl+W: deletes the word immediately before the cursor.
    fn delete_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let chars: Vec<char> = self.input.chars().collect();
        let mut idx = self.cursor;
        while idx > 0 && chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        while idx > 0 && !chars[idx - 1].is_whitespace() {
            idx -= 1;
        }
        let start = self.byte_index(idx);
        let end = self.byte_index(self.cursor);
        self.input.replace_range(start..end, "");
        self.cursor = idx;
    }
}

/// A file to fetch and the folder to put it in -- what `/save`, `Enter` in
/// the files overlay and auto-save all hand to main.rs once they have decided
/// to download one, so the three can only differ in `mode`.
///
/// `channel`, `hash`, `filename`, `providers` and `dir` are worked out by
/// `AppState::save_request` (a save someone asked for) or
/// `AppState::auto_save` (one nobody did). `providers` are who to ask for it,
/// best bet first: the file's sender, then the other peers known in its
/// channel (see `providers_for`) -- for an automatic save, only those we
/// trust. `dir` is the folder it is saved into: `channel`'s, under the files
/// root (`files::channel_dir`). `channel` itself, its stored name, also rides
/// along so that once the file has landed it can be recorded as saved *for
/// that channel* (`saved_files`). The caller picks the file's name inside
/// `dir` (`files::resolve_destination`) and performs the download
/// (`net::Net::save_file`); app.rs never touches the filesystem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveRequest {
    pub channel: String,
    pub hash: Hash,
    pub filename: String,
    pub providers: Vec<[u8; 32]>,
    pub dir: PathBuf,
    pub mode: SaveMode,
}

/// An action for the caller (`main.rs`) to perform in response to a
/// keypress, since `AppState` itself never does network I/O.
pub enum InputAction {
    /// Broadcast a composed message to the named channel.
    Send(String, ChatMessage),
    /// Join (or create) a channel from a bare name or an invite ticket
    /// string, exactly as the user typed it after `/join `.
    Join(String),
    /// Starts a brand-new private 2-person channel for `/msg`ing a peer
    /// we don't already have a joined DM channel with (see `run_msg`):
    /// `channel` is the freshly chosen, not-yet-joined local name, and
    /// `peer` is who it's for, so the caller can record the association
    /// in `crate::dm_registry` once the join actually succeeds (see
    /// `net::NetEvent::Joined`). `text`, if given, is sent once the
    /// channel is ready.
    StartDm {
        peer: [u8; 32],
        channel: String,
        text: Option<String>,
    },
    /// Starts (or resumes) a point-to-point conversation with `peer` --
    /// see `run_dm`. Unlike `StartDm`, the caller (main.rs) never needs to
    /// create the tab itself first: `net::Net::open_direct` is fully
    /// synchronous (no gossip-style subscribe to await), and the
    /// recipient side converges on the same tab via
    /// `net::NetEvent::DirectJoined` regardless of who connects first, so
    /// `run_dm` only needs to say who this is for and what (if anything)
    /// to send once ready.
    OpenDirect {
        peer: [u8; 32],
        text: Option<String>,
    },
    /// Build an invite ticket for the named channel and report it back
    /// (via `AppState::push_system`) once built.
    Invite(String),
    /// Build a pairing ticket for a brand-new device to redeem (see
    /// `net::Net::create_pairing_ticket`) and report it back (via
    /// `AppState::push_system`) once built, mirroring `Invite`.
    Pair,
    /// Stop trusting a specific device id, resolved from a typed
    /// hex-prefix by `/forget-device` (see `run_command` and
    /// `crate::forgotten_devices`). Unlike `Alias`, never canonicalized --
    /// forgetting targets one specific device, not a whole person.
    ForgetDevice([u8; 32]),
    /// Persist a local pet name for a specific endpoint id, resolved from
    /// a typed hex-prefix by `/alias` (see `run_command` and
    /// `crate::contacts`).
    Alias([u8; 32], String),
    /// Persist a local display name for a group channel -- or, with `None`,
    /// forget it -- from `/rename` (see `run_rename` and
    /// `crate::channel_renames`). Already applied to `AppState`; the caller
    /// only needs to persist it.
    Rename {
        channel: String,
        name: Option<String>,
    },
    /// Broadcast a chosen display nickname to every joined channel, as
    /// typed after `/nick` (see `run_command` and
    /// `crate::net::Net::set_nickname`).
    Nick(String),
    /// Leave a previously joined channel -- already resolved to a
    /// concrete, currently-joined name by `run_leave` (either explicitly
    /// typed after `/leave`, or the active channel if omitted). The
    /// caller is responsible for actually tearing down its gossip
    /// subscription and persisted state (see `crate::net::Net::leave`,
    /// `crate::channel_registry`, `crate::storage`, `crate::backfill`)
    /// and then calling `AppState::remove_channel`.
    Leave(String),
    /// Kick off a background scan of `channel`'s full on-disk log for
    /// `term` (already applied synchronously to the loaded scrollback by
    /// `run_search`) -- see `search::run_on_disk_scan`,
    /// `AppState::apply_search_outcome`, and main.rs.
    Search {
        channel: String,
        term: String,
    },
    /// Share a local file with `channel` -- `path` exactly as typed after
    /// `/send ` (see `run_send`). The caller (`net::Net::send_file`)
    /// validates and imports it; app.rs never touches the filesystem.
    SendFile {
        channel: String,
        path: String,
    },
    /// Share everything staged on `channel` (see `Channel::staged`), with
    /// `caption` -- whatever was typed when `Enter` was pressed, empty for
    /// none -- on the first attachment. The caller
    /// (`net::Net::send_staged`) imports and announces each one, much as
    /// for `SendFile`; app.rs never touches the filesystem. The stage is
    /// already empty by the time this is returned.
    SendStaged {
        channel: String,
        items: Vec<StagedAttachment>,
        caption: String,
    },
    /// Fetch a previously shared file and save it locally -- all of it
    /// already resolved by `save_request`, for `run_save` (from a typed
    /// hash-prefix -- see `resolve_attachment` -- or, with none, as the
    /// newest file in the active channel -- see `newest_shared_file`) and for
    /// `Enter` in the files overlay. See `SaveRequest`. (Auto-save hands the
    /// caller a `SaveRequest` directly, from `auto_save`.)
    SaveFile(SaveRequest),
    /// Remember that auto-save is now on or off for `channel` (a stored
    /// name), from `/autosave` (see `run_autosave`). Already applied to
    /// `AppState`; the caller only needs to persist it
    /// (`autosave::AutoSave::set`).
    SetAutoSave {
        channel: String,
        on: bool,
    },
    SetChannelAutoSaveLimit {
        channel: String,
        limit: Option<AutoSaveLimit>,
    },
    /// Create or remove the active channel's one-way outbox setting. The
    /// runtime creates and snapshots the folder before applying this state.
    SetOutbox {
        channel: String,
        on: bool,
    },
    /// Persist folder sync for one channel. Application of remote entries
    /// and publication of stable local files remain runtime-owned.
    SetSync {
        channel: String,
        config: SyncConfig,
    },
    /// Inspect recoverable remote deletes for the active channel. The
    /// runtime performs the filesystem scan and reports it back.
    ListSyncTrash(String),
    /// Restore one relative path from the active channel's sync trash.
    RestoreSyncTrash {
        channel: String,
        path: String,
    },
    /// Permanently delete every recoverable remote delete in the active
    /// channel after this explicit command.
    PurgeSyncTrash(String),
    ListSyncActivity(String),
    ResolveSyncConflict {
        channel: String,
        conflict: SyncConflict,
        resolution: ConflictResolution,
    },
    /// Move the files root -- the folder every channel's folder lives in --
    /// to the path exactly as typed after `/folder ` (see `run_folder`). The
    /// caller (`files::set_files_root`) expands, creates and remembers it,
    /// then reports back through `AppState::load_files_root`: app.rs never
    /// touches the filesystem, so it can't tell whether the path will do.
    SetFilesRoot(String),
    /// Remember a new size limit across restarts, from `/maxsize` (see
    /// `run_maxsize`). Already applied to `AppState`, and already validated:
    /// the caller only needs to persist it (`files::set_size_limit`), as it
    /// does for `Bell`.
    SetSizeLimit(SizeLimit),
    /// Read the local folder for an open `/files` overlay on a blocking
    /// thread, then return a `FilesListing`. The request identifies the
    /// overlay that asked for it.
    ReadFiles {
        request: u64,
        channel: String,
        dir: PathBuf,
    },
    /// Open the saved file at `path` with the OS's default application --
    /// `Ctrl+O` in the files overlay (`open_highlighted_file`). Never for a
    /// file that runs something when opened (`files::runs_when_opened`):
    /// the overlay refuses those, so the caller hands over whatever it is
    /// given -- once it has checked the file is still there
    /// (`files::open_in_default_app`).
    OpenFile(PathBuf),
    /// Reveal a local file in the operating system's file manager.
    RevealFile(PathBuf),
    /// Open a local file in the operating system's text editor.
    EditFile(PathBuf),
    /// Share whatever's on the OS clipboard with `channel` -- from `/paste`
    /// or the `Ctrl+V`/`Cmd+V` keybinding (see `run_paste`). The caller
    /// (`net::Net::paste`) reads the clipboard and decides what it found;
    /// app.rs never touches the clipboard directly, matching this module's
    /// doc comment.
    Paste(String),
    /// Persist a new bell-on-incoming-message preference, toggled via
    /// `/bell` (see `run_bell` and `crate::settings::Settings`). Applied
    /// immediately to `bell_enabled` for instant feedback; the caller only
    /// needs to persist it to disk.
    Bell(bool),
    /// Persist a new mouse-reporting preference, toggled via `/mouse` (see
    /// `run_mouse`). Like `Bell`, `mouse_enabled` is already updated; the
    /// caller only persists it, while the main loop's `MouseReporting`
    /// follows `mouse_enabled` on its own.
    Mouse(bool),
    /// Read `dir` for the file picker (`picker::read_folder`, off the event
    /// loop) and report it back as a `picker::FolderListing` for `request`,
    /// which `AppState::apply_folder_listing` matches to the read it was
    /// waiting for. app.rs never touches the filesystem, so the read is the
    /// caller's.
    ReadFolder {
        request: u64,
        dir: PathBuf,
    },
    /// Stage `paths`, chosen in the file picker, on `channel` -- each
    /// checked like a `/send`ed file (`files::stage_file`) -- and remember
    /// `folder` as where the picker opens next time
    /// (`picker::remember_folder`).
    StageFiles {
        channel: String,
        paths: Vec<PathBuf>,
        folder: PathBuf,
    },
}

/// All state needed to render the TUI and respond to input.
pub struct AppState {
    /// Our own identity, so we can tell our messages apart from peers'.
    pub self_id: [u8; 32],
    /// Our own canonical user identity -- the shared user key's public
    /// half (see `crate::net::Net::our_user_id` and `crate::message::
    /// DeviceCert::user_id`), seeded once via `load_self_user_id` right
    /// after `AppState::new`, before the event loop starts, mirroring
    /// `load_contacts`. Lets `canonical_id` recognize a peer broadcasting
    /// a certificate under this same user id as one of our *own* other
    /// devices rather than an unrelated stranger -- see that method's and
    /// `display_name`/`unique_peer_ids`'s doc comments, both of which rely
    /// on it. Defaults to `self_id` until loaded; that can never collide
    /// with a real peer's canonical id, since a cert claiming our own
    /// device id as someone else's user id would fail `DeviceCert::
    /// is_valid` before ever reaching `device_to_user`.
    self_user_id: [u8; 32],
    pub channels: Vec<Channel>,
    pub active: usize,
    pub should_quit: bool,
    /// Whether the sidebar's command hints panel is shown -- toggled via
    /// `/hints` (`run_hints`). Visible by default so the panel actually
    /// teaches new users; in-memory only, like `scroll`, not persisted
    /// across restarts.
    pub show_hints: bool,
    /// Whether the terminal reports the mouse to us, which hover popups
    /// over command hints and peers need -- toggled via `/mouse`
    /// (`run_mouse`). On by default and persisted like `bell_enabled`;
    /// while it's on, native text selection needs a modifier (Shift, or
    /// Option in iTerm2).
    pub mouse_enabled: bool,
    /// The command-hints row under the mouse, if any -- `ui::render`
    /// shows that command's popup while it's set.
    hint_hover: Option<usize>,
    /// The peers-panel peer under the mouse, if any -- `ui::render`
    /// shows their popup while it's set.
    peer_hover: Option<[u8; 32]>,
    /// Whether an incoming message rings the terminal bell -- toggled via
    /// `/bell` (`run_bell`). Unlike `show_hints`, this preference is
    /// persisted across restarts (see `crate::settings::Settings`);
    /// `AppState::new` just starts at the same default (`true`) until
    /// `load_settings` seeds the actual saved value, mirroring how
    /// `petnames` starts empty until `load_contacts` runs.
    pub bell_enabled: bool,
    /// This device's persisted broadcast nickname, if one was configured
    /// with `/nick`. Kept separately from peer announcements because this
    /// device does not receive its own identity gossip.
    local_nickname: Option<String>,
    /// Local pet names assigned via `/alias`, keyed by endpoint id --
    /// checked by `display_name` before falling back to a hex prefix.
    /// Seeded once at startup from `crate::contacts::Contacts` (see
    /// `load_contacts`); never sent over the wire.
    petnames: HashMap<[u8; 32], String>,
    /// Last-seen broadcast nickname per sender, from `/nick` (see
    /// `NetEvent::Identity`) -- checked by `display_name` after `petnames`
    /// but before the hex-prefix fallback. In-memory only, unlike
    /// `petnames`: a broadcast nickname isn't ours to persist, and its
    /// owner re-announces it on every reconnect anyway.
    nicknames: HashMap<[u8; 32], String>,
    /// Last-announced `(version, git_hash)` per sender, from
    /// `NetEvent::Version` -- checked by `who_line` and
    /// `newest_known_update`. In-memory only, like `nicknames`: it isn't
    /// ours to persist, and each peer re-announces it on every reconnect
    /// anyway.
    peer_versions: HashMap<[u8; 32], (String, String)>,
    /// Device id -> user id, built from verified `crate::message::DeviceCert`s
    /// (`NetEvent::Device`, already checked by `net::forward_events` before
    /// it ever reaches here). Consulted by `canonical_id`, which is what
    /// actually lets two of one person's devices collapse onto a single
    /// display identity. In-memory only, like `nicknames`/`peer_versions`:
    /// a peer re-announces their cert on every reconnect anyway.
    device_to_user: HashMap<[u8; 32], [u8; 32]>,
    /// Device ids explicitly forgotten via `/forget-device` -- see
    /// `crate::forgotten_devices`. Consulted by `canonical_id`, which
    /// treats a forgotten device exactly as if no certificate were known
    /// for it, so forgetting a device immediately stops it from being
    /// grouped under its (former) canonical user id as well as from being
    /// trusted for future device-sync announces (see main.rs). Local and
    /// best-effort, not real revocation -- see that module's doc comment.
    forgotten: HashSet<[u8; 32]>,
    /// Peer id -> local DM channel name, so `/msg` can tell whether a peer
    /// already has a private 1:1 channel to reuse -- see `run_msg` and
    /// `crate::dm_registry`. Seeded once at startup from
    /// `crate::dm_registry::DmRegistry` (see `load_dm_registry`), then kept
    /// current as new DMs are started or accepted this session (see
    /// `NetEvent::DmJoined` and `run_msg`'s creation path).
    dm_channels: HashMap<[u8; 32], String>,
    /// Peer id -> local point-to-point channel name (see `direct.rs`),
    /// mirroring `dm_channels`'s role for `/msg` but for `/dm`'s
    /// point-to-point conversations instead of gossip-based ones. Unlike
    /// `dm_channels`, the name is always `direct::channel_name(&peer)` --
    /// deterministic, not cosmetic -- so this map exists purely to answer
    /// "do we already have a tab for this peer" (`run_dm`) and "which
    /// peer does this channel name belong to" (`direct_peer_for`, used by
    /// main.rs to route a send to `net::Net::send_direct` instead of
    /// `net::Net::send`). Populated either synchronously when we initiate
    /// (`activate_direct_channel`) or via `NetEvent::DirectJoined` when a
    /// peer reaches us first (`background_direct_channel`).
    direct_channels: HashMap<[u8; 32], String>,
    /// Stored channel name -> local physical folder name for direct chats.
    /// Group channels deliberately have no entry and continue to derive
    /// their folder from the immutable channel name.
    direct_folders: HashMap<String, String>,
    shared_files: HashMap<String, Vec<SharedFile>>,
    /// Local display names given to group channels via `/rename`, keyed by
    /// the channel's stored name (its identity, which never changes) --
    /// checked by `channel_label`, the channel counterpart to `petnames`.
    /// Seeded once at startup from `crate::channel_renames::ChannelRenames`
    /// (see `load_renames`); never sent over the wire, and dropped with the
    /// channel by `remove_channel` so a later channel under the same stored
    /// name doesn't inherit it.
    renames: HashMap<String, String>,
    /// The in-progress candidate while picking a reply target with
    /// `Ctrl+R` (see `start_reply_pick`/`move_reply_pick`), by message id.
    /// `None` when not picking -- `handle_key` checks this first, before
    /// normal typing/editing, to dispatch into `handle_picking_key`.
    pub picking_reply: Option<u64>,
    /// The in-progress candidate while picking a message to view its
    /// thread with `Ctrl+T` (see `start_thread_pick`/`move_thread_pick`),
    /// by message id. `None` when not picking -- `handle_key` checks this
    /// alongside `picking_reply`, before normal typing/editing, to
    /// dispatch into `handle_thread_picking_key`.
    pub picking_thread: Option<u64>,
    /// The open jump-to-conversation overlay (`Ctrl+P`, see
    /// `start_switcher`), if any. `None` when closed -- `handle_key`
    /// checks this alongside `picking_reply`/`picking_thread`, before
    /// normal typing/editing, to dispatch into `handle_switcher_key`.
    pub switching: Option<Switcher>,
    /// The open slash-command picker (`Ctrl+K`). It owns keyboard input like
    /// the conversation switcher and never invokes a command directly.
    pub command_picker: Option<CommandPicker>,
    /// The peer-details overlay opened with `/who`. It owns input until
    /// `Esc` closes it, like the file and command-picker overlays.
    pub viewing_peers: Option<PeersOverlay>,
    /// The open file picker (a bare `/send`, see `run_send`), if any. A modal
    /// mode like `switching`: `handle_key` hands it every key, a paste
    /// filters it, and `is_composing` is false while it's open.
    pub picking_file: Option<FilePicker>,
    /// The open files overlay (`/files`, see `run_files`), if any. A modal
    /// mode like `switching`: `handle_key` hands it every key, a paste
    /// filters it, and `is_composing` is false while it's open.
    pub viewing_files: Option<FilesOverlay>,
    /// The folder the file picker opens in: where files were last chosen
    /// from -- seeded at startup from what was remembered across restarts
    /// (`load_browse_dir`) and updated whenever files are chosen. The
    /// filesystem root until seeded, so there is always somewhere to open.
    browse_dir: PathBuf,
    /// The files root: the folder every channel's folder of saved files
    /// lives in (`files::channel_dir`) -- seeded at startup from what was
    /// remembered across restarts (`load_files_root`) and replaced when
    /// `/folder <path>` takes effect. The filesystem root until seeded, like
    /// `browse_dir`, so a save can never land in the working directory.
    files_root: PathBuf,
    /// How big a file may be to share -- seeded at startup from what was
    /// remembered across restarts (`load_size_limit`) and replaced when
    /// `/maxsize` takes effect (`run_maxsize`). The default until seeded.
    /// main.rs reads it (`size_limit`) wherever a file is checked for
    /// sharing, so every route a file takes -- `/send <path>`, the picker, a
    /// drop, the clipboard -- answers to the same one.
    size_limit: SizeLimit,
    autosave_limit: AutoSaveLimit,
    autosave_limits: HashMap<String, AutoSaveLimit>,
    /// The channels, by stored name, with auto-save on (`/autosave`) --
    /// seeded at startup from what was remembered across restarts
    /// (`load_autosave`) and changed by `/autosave on|off`. A channel not in
    /// here never downloads anything by itself, which is how every channel
    /// starts. Keyed by stored name like the channel's folder is, so
    /// `remove_channel` forgets it: a ticket's name is chosen by whoever
    /// shared it, and a setting that outlived its channel would switch
    /// downloads on for a different room that reuses the name.
    autosave: HashSet<String>,
    /// The channels whose local `send/` folder is watched for outbound
    /// files. `outbox.rs` owns durable settings and de-duplication; app
    /// state only exposes this enabled set to `/outbox`.
    outbox: HashSet<String>,
    /// Persisted per-channel sync policy and path rules. App state only
    /// exposes the configuration; runtime owns filesystem and network work.
    sync: HashMap<String, SyncConfig>,
    /// Runtime-owned sync observations supplied through explicit events.
    /// This state never reads the filesystem or network itself.
    sync_status: HashMap<String, SyncStatus>,
    /// The files an auto-save is fetching right now, by channel (stored
    /// name) and content. A repeat of one of them -- a file sent twice, or a
    /// share replayed by someone who can't be told from its sender -- is
    /// dropped instead of starting a download per copy, which would leave a
    /// `name (n).ext` for each. Not persisted: a download doesn't survive the
    /// process. Cleared when the save finishes (`NetEvent::FileSaved`,
    /// `save_failed`) and when the channel is left.
    auto_saving: HashSet<(String, Hash)>,
    /// Counts the folder reads the picker has asked for (`ReadFolder`), so
    /// every one has an id of its own -- across pickers too, which is what
    /// keeps a slow answer to a closed picker's read from landing in the
    /// next one.
    browse_requests: u64,
    /// Sequence for `/files` folder snapshots, separate from send-file
    /// picker reads so replies cannot be applied to the wrong overlay.
    files_requests: u64,
    /// The armed reply target for the next sent message (confirmed via
    /// `Enter` while picking, or `/reply`), by message id. Attached to the
    /// next `Send`-ed `ChatMessage` and cleared on send (`send_text`), on
    /// `Esc` (`handle_key`), or whenever the active channel changes
    /// (`switch_channel`, `remove_channel`, `NetEvent::Joined`) -- a reply
    /// id only makes sense against the channel it was picked in.
    pub replying_to: Option<u64>,
    /// A live `/pair <ticket>` redeem awaiting explicit confirmation (see
    /// `run_pair`/`handle_redeem_confirmation_key`), already known to
    /// decode successfully -- armed the moment the warning is shown, taken
    /// (on `Enter`) into `restart_with_ticket`, or cleared outright (on
    /// `Esc`). Checked first in `handle_key`, like `picking_reply`/
    /// `picking_thread`, since confirming this is destructive enough to
    /// warrant ignoring every other key while it's armed.
    pending_redeem: Option<String>,
    /// Set once a live pairing redeem is actually confirmed (`Enter` while
    /// `pending_redeem` is armed) -- alongside `should_quit`, so the normal
    /// quit path (main.rs's event loop ending, `Net::shutdown`, terminal
    /// restore) runs unchanged first. main.rs checks this once the loop
    /// exits to decide whether to wipe local state and restart into
    /// `--pair <ticket>` instead of just exiting -- see main.rs's `run`.
    pub restart_with_ticket: Option<String>,
}

impl AppState {
    /// Builds the initial state for a session with the given identity and
    /// already-joined channel names (in join order). `active_channel`
    /// selects which one starts active -- e.g. so `--join
    /// <ticket-for-project-x>` lands you in `project-x`, not `general` --
    /// falling back to the first channel if `active_channel` isn't among
    /// `channel_names`.
    ///
    /// # Panics
    /// Panics if `channel_names` is empty. `AppState` always has at least
    /// one channel, so `active` is always a valid index.
    pub fn new(self_id: [u8; 32], channel_names: Vec<String>, active_channel: &str) -> Self {
        assert!(
            !channel_names.is_empty(),
            "AppState must start with at least one channel"
        );
        let active = channel_names
            .iter()
            .position(|name| name == active_channel)
            .unwrap_or(0);
        let channels: Vec<Channel> = channel_names.into_iter().map(Channel::new).collect();
        Self {
            self_id,
            self_user_id: self_id,
            channels,
            active,
            should_quit: false,
            show_hints: true,
            mouse_enabled: true,
            hint_hover: None,
            peer_hover: None,
            bell_enabled: true,
            local_nickname: None,
            petnames: HashMap::new(),
            nicknames: HashMap::new(),
            peer_versions: HashMap::new(),
            device_to_user: HashMap::new(),
            forgotten: HashSet::new(),
            dm_channels: HashMap::new(),
            direct_channels: HashMap::new(),
            direct_folders: HashMap::new(),
            shared_files: HashMap::new(),
            renames: HashMap::new(),
            picking_reply: None,
            picking_thread: None,
            switching: None,
            command_picker: None,
            viewing_peers: None,
            replying_to: None,
            pending_redeem: None,
            restart_with_ticket: None,
            picking_file: None,
            viewing_files: None,
            browse_dir: PathBuf::from(std::path::MAIN_SEPARATOR_STR),
            files_root: PathBuf::from(std::path::MAIN_SEPARATOR_STR),
            size_limit: SizeLimit::Default,
            autosave_limit: AutoSaveLimit::Default,
            autosave_limits: HashMap::new(),
            autosave: HashSet::new(),
            outbox: HashSet::new(),
            sync: HashMap::new(),
            sync_status: HashMap::new(),
            auto_saving: HashSet::new(),
            browse_requests: 0,
            files_requests: 0,
        }
    }
}

mod commands;
mod compose;
mod file_actions;
mod identity;
mod input;
mod state;
mod sync;

/// The peer `associations` (peer id -> channel name, as `dm_channels` and
/// `direct_channels` both are) maps to `channel`, if any -- the reverse
/// lookup `AppState::channel_kind` classifies by. Linear, which is fine:
/// each map holds one entry per 1:1 conversation, a handful at most.
fn peer_for_channel(associations: &HashMap<[u8; 32], String>, channel: &str) -> Option<[u8; 32]> {
    associations
        .iter()
        .find(|(_, name)| name.as_str() == channel)
        .map(|(peer, _)| *peer)
}

/// Shortened hex id used to display peers we don't have a nickname for.
pub fn hex_prefix(bytes: &[u8; 32]) -> String {
    bytes[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Full hex id, e.g. for display in the header so it can be copied into
/// another instance's `--connect` flag.
pub fn hex_id(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Decodes a full, 64-character hex endpoint id (as typed after `/dm`) into
/// raw bytes -- the inverse of `hex_id`. `None` for anything else (wrong
/// length, non-hex characters), including a mere hex *prefix* -- only a
/// complete id can be dialed with no other information, so a partial one
/// is never silently accepted here (see `resolve_dm_target`).
fn decode_hex_id(target: &str) -> Option<[u8; 32]> {
    if target.len() != 64 || !target.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let mut id = [0u8; 32];
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&target[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(id)
}

/// Strips one layer of matching leading/trailing quotes from a `/send`
/// argument, e.g. `"~/Downloads/file.png"` -- there's no shell here to do
/// that for us, and some copy/paste or drag-and-drop sources wrap a path
/// in quotes out of habit. Left as-is if the quotes don't match (or
/// there's only one).
fn strip_quotes(arg: &str) -> &str {
    let bytes = arg.as_bytes();
    let quoted = bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''));
    if quoted { &arg[1..arg.len() - 1] } else { arg }
}

/// The command that downloads the file `hash` by hand -- what a notice about
/// a file that was not saved automatically points to, in the words the
/// transcript's own line for the file uses (`ui::attachment_line`).
fn save_hint(hash: Hash) -> String {
    format!("/save {}", &hash.to_hex()[..8])
}

/// The next id of `counter`, which counts the folder reads the file picker
/// has asked for (`AppState::browse_requests`).
fn next_request(counter: &mut u64) -> u64 {
    *counter += 1;
    *counter
}

fn compose_message(sender: [u8; 32], text: &str, reply_to: Option<u64>) -> ChatMessage {
    ChatMessage {
        v: 3,
        id: rand::random(),
        sender,
        ts_unix_ms: now_unix_ms(),
        text: text.to_string(),
        attachment: None,
        reply_to,
    }
}

#[cfg(test)]
mod tests;
