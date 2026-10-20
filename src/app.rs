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

/// Cap on in-memory scrollback per channel, per concept.md's guidance on
/// bounding render/memory cost in a long-lived session.
const MAX_SCROLLBACK: usize = 500;

/// One user-facing slash command. This catalog is the shared source for the
/// command picker and sidebar hints; `input` is what selecting it places in
/// the compose draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CommandDefinition {
    pub syntax: &'static str,
    pub input: &'static str,
    /// A global keybinding for this command or its more specific picker, if
    /// one exists.
    pub shortcut: Option<&'static str>,
    pub aliases: &'static [&'static str],
    pub description: &'static str,
}

const COMMANDS: &[CommandDefinition] = &[
    CommandDefinition {
        syntax: "/join <name|ticket>",
        input: "/join ",
        shortcut: None,
        aliases: &[],
        description: "Join or create a channel",
    },
    CommandDefinition {
        syntax: "/invite",
        input: "/invite",
        shortcut: None,
        aliases: &[],
        description: "Show an invite for this channel",
    },
    CommandDefinition {
        syntax: "/pair [ticket]",
        input: "/pair ",
        shortcut: None,
        aliases: &[],
        description: "Pair a device or show a pairing ticket",
    },
    CommandDefinition {
        syntax: "/forget-device <hex-prefix>",
        input: "/forget-device ",
        shortcut: None,
        aliases: &[],
        description: "Forget a paired device",
    },
    CommandDefinition {
        syntax: "/msg <peer> [text]",
        input: "/msg ",
        shortcut: None,
        aliases: &[],
        description: "Start a private channel message",
    },
    CommandDefinition {
        syntax: "/dm <peer> [text]",
        input: "/dm ",
        shortcut: None,
        aliases: &[],
        description: "Start a direct peer conversation",
    },
    CommandDefinition {
        syntax: "/alias <hex-prefix> <name>",
        input: "/alias ",
        shortcut: None,
        aliases: &[],
        description: "Set a local name for a peer",
    },
    CommandDefinition {
        syntax: "/nick <name>",
        input: "/nick ",
        shortcut: None,
        aliases: &[],
        description: "Set your broadcast display name",
    },
    CommandDefinition {
        syntax: "/leave [channel]",
        input: "/leave",
        shortcut: None,
        aliases: &[],
        description: "Leave a channel",
    },
    CommandDefinition {
        syntax: "/rename [name]",
        input: "/rename ",
        shortcut: None,
        aliases: &[],
        description: "Set this channel's local display name",
    },
    CommandDefinition {
        syntax: "/who",
        input: "/who",
        shortcut: None,
        aliases: &[],
        description: "View this device and active peer details",
    },
    CommandDefinition {
        syntax: "/send [path]",
        input: "/send",
        shortcut: None,
        aliases: &[],
        description: "Pick or share a file",
    },
    CommandDefinition {
        syntax: "/save [hash-prefix]",
        input: "/save",
        shortcut: None,
        aliases: &[],
        description: "Download a shared file",
    },
    CommandDefinition {
        syntax: "/folder [path]",
        input: "/folder",
        shortcut: None,
        aliases: &[],
        description: "Show or set the files folder",
    },
    CommandDefinition {
        syntax: "/maxsize [size|off|default]",
        input: "/maxsize",
        shortcut: None,
        aliases: &[],
        description: "Show or set the file size limit",
    },
    CommandDefinition {
        syntax: "/autosave [on|off|limit]",
        input: "/autosave",
        shortcut: None,
        aliases: &[],
        description: "Configure automatic file saving",
    },
    CommandDefinition {
        syntax: "/outbox [on|off]",
        input: "/outbox",
        shortcut: None,
        aliases: &[],
        description: "Configure the watched send folder",
    },
    CommandDefinition {
        syntax: "/sync [options]",
        input: "/sync",
        shortcut: None,
        aliases: &[],
        description: "Configure folder synchronization",
    },
    CommandDefinition {
        syntax: "/activity",
        input: "/activity",
        shortcut: None,
        aliases: &[],
        description: "Show recent sync activity",
    },
    CommandDefinition {
        syntax: "/conflicts [resolution]",
        input: "/conflicts",
        shortcut: None,
        aliases: &[],
        description: "List or resolve sync conflicts",
    },
    CommandDefinition {
        syntax: "/trash [restore|purge]",
        input: "/trash",
        shortcut: None,
        aliases: &[],
        description: "Manage recoverable sync deletes",
    },
    CommandDefinition {
        syntax: "/files",
        input: "/files",
        shortcut: None,
        aliases: &[],
        description: "Browse shared and synced channel files",
    },
    CommandDefinition {
        syntax: "/paste",
        input: "/paste",
        shortcut: Some("Ctrl+V"),
        aliases: &[],
        description: "Share the clipboard",
    },
    CommandDefinition {
        syntax: "/search <term>",
        input: "/search ",
        shortcut: None,
        aliases: &["/s"],
        description: "Search channel messages",
    },
    CommandDefinition {
        syntax: "/reply [text]",
        input: "/reply",
        shortcut: Some("Ctrl+R"),
        aliases: &[],
        description: "Reply to the latest message",
    },
    CommandDefinition {
        syntax: "/thread",
        input: "/thread",
        shortcut: Some("Ctrl+T"),
        aliases: &[],
        description: "View the latest reply thread",
    },
    CommandDefinition {
        syntax: "/hints",
        input: "/hints",
        shortcut: None,
        aliases: &[],
        description: "Toggle command hints and hover reporting",
    },
    CommandDefinition {
        syntax: "/bell",
        input: "/bell",
        shortcut: None,
        aliases: &[],
        description: "Toggle the message bell",
    },
    CommandDefinition {
        syntax: "/version",
        input: "/version",
        shortcut: None,
        aliases: &[],
        description: "Show this build's version",
    },
    CommandDefinition {
        syntax: "/help",
        input: "/help",
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
    pub peers: Vec<[u8; 32]>,
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

    /// Seeds `channel`'s transcript from persisted history loaded by the
    /// caller (see storage.rs). A no-op if `channel` isn't currently
    /// joined. Intended to be called once per joined channel right after
    /// `AppState::new`, before the event loop starts.
    pub fn load_history(&mut self, channel: &str, messages: Vec<ChatMessage>) {
        for message in &messages {
            if let Some(file) = &message.attachment {
                self.record_shared_file(
                    channel,
                    SharedFile {
                        message_id: message.id,
                        file: file.clone(),
                        sender: message.sender,
                        ts_unix_ms: message.ts_unix_ms,
                    },
                );
            }
        }
        if let Some(channel) = self.channel_mut(channel) {
            channel.seed_history(messages);
        }
    }

    /// Seeds local pet names persisted by the caller (see contacts.rs).
    /// Intended to be called once, right after `AppState::new`, before
    /// the event loop starts -- mirrors `load_history`.
    pub fn load_contacts(&mut self, petnames: HashMap<[u8; 32], String>) {
        self.petnames = petnames;
    }
    /// Seeds remembered peer observations loaded by the caller. Live gossip
    /// events still decide online state; this data only keeps peers visible
    /// after they leave and across restarts.
    pub fn load_last_seen(&mut self, last_seen: HashMap<String, HashMap<[u8; 32], u64>>) {
        for channel in &mut self.channels {
            if let Some(peers) = last_seen.get(&channel.name) {
                channel.last_seen = peers.clone();
            }
        }
    }

    /// Records a peer observation in the in-memory state. The runtime owns
    /// persistence and calls this alongside `LastSeen::record`.
    pub fn record_peer_seen(&mut self, channel: &str, id: [u8; 32], seen_unix_ms: u64) {
        if let Some(channel) = self.channel_mut(channel) {
            channel.last_seen.insert(id, seen_unix_ms);
        }
    }

    /// Seeds our own canonical user identity (see `self_user_id`), loaded
    /// by the caller from `net::Net::our_user_id`. Intended to be called
    /// once, right after `AppState::new`, before the event loop starts --
    /// mirrors `load_contacts`.
    pub fn load_self_user_id(&mut self, user_id: [u8; 32]) {
        self.self_user_id = user_id;
    }

    /// Seeds persisted local preferences loaded by the caller (see
    /// settings.rs). Intended to be called once, right after
    /// `AppState::new`, before the event loop starts -- mirrors
    /// `load_contacts`.
    pub fn load_settings(&mut self, bell_enabled: bool, nickname: Option<String>) {
        self.bell_enabled = bell_enabled;
        self.local_nickname = nickname;
    }

    /// Seeds known peer <-> DM channel associations persisted by the
    /// caller (see `crate::dm_registry`). Intended to be called once,
    /// right after `AppState::new`, before the event loop starts --
    /// mirrors `load_contacts`.
    pub fn load_dm_registry(&mut self, dm_channels: HashMap<[u8; 32], String>) {
        self.dm_channels = dm_channels;
    }

    /// Seeds forgotten device ids persisted by the caller (see
    /// `crate::forgotten_devices`). Intended to be called once, right
    /// after `AppState::new`, before the event loop starts -- mirrors
    /// `load_contacts`.
    pub fn load_forgotten_devices(&mut self, forgotten: HashSet<[u8; 32]>) {
        self.forgotten = forgotten;
    }

    /// Seeds the local channel display names persisted by the caller (see
    /// `crate::channel_renames`). Intended to be called once, right after
    /// `AppState::new`, before the event loop starts -- mirrors
    /// `load_contacts`.
    pub fn load_renames(&mut self, renames: HashMap<String, String>) {
        self.renames = renames;
    }

    /// Seeds the folder the file picker opens in (see
    /// `picker::start_folder`, which resolves what was remembered across
    /// restarts). Intended to be called once, right after `AppState::new`,
    /// before the event loop starts -- mirrors `load_contacts`.
    pub fn load_browse_dir(&mut self, dir: PathBuf) {
        self.browse_dir = dir;
    }

    /// Seeds the files root (see `files::files_root`, which resolves what
    /// was remembered across restarts) -- and replaces it once a `/folder
    /// <path>` has taken effect (`InputAction::SetFilesRoot`). Intended to be
    /// called once, right after `AppState::new`, before the event loop
    /// starts -- mirrors `load_browse_dir`.
    pub fn load_files_root(&mut self, root: PathBuf) {
        self.files_root = root;
    }

    /// Associates one direct channel with its persisted local folder name.
    /// The caller owns persistence; app state only resolves paths from the
    /// already validated mapping.
    pub fn load_direct_folder(&mut self, channel: String, folder: String) {
        self.direct_folders.insert(channel, folder);
    }

    pub fn load_shared_files(&mut self, files: HashMap<String, Vec<SharedFile>>) {
        self.shared_files = files;
    }

    pub fn record_shared_file(&mut self, channel: &str, shared: SharedFile) {
        let files = self.shared_files.entry(channel.to_string()).or_default();
        if files
            .iter()
            .any(|known| known.message_id == shared.message_id)
        {
            return;
        }
        files.push(shared);
        files.sort_by_key(|shared| std::cmp::Reverse((shared.ts_unix_ms, shared.message_id)));
    }

    /// The physical folder for `channel`. Direct mappings are local-only;
    /// every other channel retains its original stored-name folder.
    pub fn folder_dir(&self, channel: &str) -> PathBuf {
        let folder = self
            .direct_folders
            .get(channel)
            .map_or(channel, String::as_str);
        channel_dir(&self.files_root, folder)
    }

    /// Seeds the size limit (see `files::size_limit`, which resolves what
    /// was remembered across restarts). Intended to be called once, right
    /// after `AppState::new`, before the event loop starts -- mirrors
    /// `load_files_root`.
    pub fn load_size_limit(&mut self, limit: SizeLimit) {
        self.size_limit = limit;
    }

    pub fn load_autosave_limit(&mut self, limit: AutoSaveLimit) {
        self.autosave_limit = limit;
    }
    pub fn load_autosave_limits(&mut self, limits: HashMap<String, AutoSaveLimit>) {
        self.autosave_limits = limits;
    }
    fn autosave_limit_for(&self, channel: &str) -> AutoSaveLimit {
        self.autosave_limits
            .get(channel)
            .copied()
            .unwrap_or(self.autosave_limit)
    }

    /// Seeds the channels with auto-save on (see `autosave::AutoSave`, which
    /// holds what was remembered across restarts). Intended to be called
    /// once, right after `AppState::new`, before the event loop starts --
    /// mirrors `load_renames`.
    pub fn load_autosave(&mut self, channels: HashSet<String>) {
        self.autosave = channels;
    }

    /// Seeds the enabled one-way outboxes persisted by the caller.
    pub fn load_outbox(&mut self, channels: HashSet<String>) {
        self.outbox = channels;
    }

    pub fn load_sync(&mut self, configs: HashMap<String, SyncConfig>) {
        self.sync = configs;
    }

    pub fn sync_config(&self, channel: &str) -> SyncConfig {
        self.sync.get(channel).cloned().unwrap_or_default()
    }
    pub fn sync_receives(&self, channel: &str) -> bool {
        self.sync_config(channel).receives()
    }
    pub fn sync_paused(&self, channel: &str) -> bool {
        self.sync_config(channel).is_paused()
    }
    pub fn sync_allows_path(&self, channel: &str, path: &str) -> bool {
        self.sync_config(channel).allows_path(path)
    }

    pub fn sync_status(&self, channel: &str) -> Option<&SyncStatus> {
        self.sync_status.get(channel)
    }
    pub fn sync_is_pending(&self, channel: &str, path: &str) -> bool {
        self.sync_status
            .get(channel)
            .is_some_and(|status| status.pending.contains(path))
    }

    pub fn sync_pending(&mut self, channel: &str, path: String) {
        self.sync_status
            .entry(channel.to_string())
            .or_default()
            .pending(path);
    }

    pub fn sync_succeeded(&mut self, channel: &str, path: &str, completed_unix_ms: u64) {
        self.sync_status
            .entry(channel.to_string())
            .or_default()
            .succeeded(path, completed_unix_ms);
    }

    pub fn sync_failed(&mut self, channel: &str, path: String, error: String) {
        self.sync_status
            .entry(channel.to_string())
            .or_default()
            .failed(path, error);
    }

    pub fn sync_conflict(&mut self, channel: &str, conflict: SyncConflict) {
        self.sync_status
            .entry(channel.to_string())
            .or_default()
            .conflict(conflict);
    }
    pub fn sync_conflicts(&self, channel: &str) -> Vec<SyncConflict> {
        self.sync_status(channel)
            .map(|status| status.conflicts().to_vec())
            .unwrap_or_default()
    }
    pub fn sync_conflict_resolved(&mut self, channel: &str, conflict: &SyncConflict) {
        if let Some(status) = self.sync_status.get_mut(channel) {
            status.conflicts.retain(|known| known != conflict);
        }
    }

    pub fn sync_marker(&self, channel: &str) -> Option<char> {
        let config = self.sync_config(channel);
        if config.policy == SyncPolicy::Off {
            None
        } else if config.is_paused() {
            Some('P')
        } else if self
            .sync_status(channel)
            .is_some_and(|status| status.error_count() > 0)
        {
            Some('!')
        } else if self
            .sync_status(channel)
            .is_some_and(|status| status.conflict_count() > 0)
        {
            Some('C')
        } else if self
            .sync_status(channel)
            .is_some_and(|status| status.pending_count() > 0)
        {
            Some('~')
        } else {
            Some('S')
        }
    }

    /// Applies a successfully persisted outbox change from the runtime.
    pub fn set_outbox(&mut self, channel: String, on: bool) {
        if on {
            self.outbox.insert(channel);
        } else {
            self.outbox.remove(&channel);
        }
    }

    /// How big a file may be to share right now (`/maxsize`).
    pub fn size_limit(&self) -> SizeLimit {
        self.size_limit
    }

    /// Merges a batch of possibly-historical messages fetched via backfill
    /// (see backfill.rs, `NetEvent::HistoryFetched`) into `channel`'s
    /// transcript. A no-op if `channel` isn't currently joined.
    ///
    /// Dedupes by id using the same `remember_seen` state as live messages.
    /// Unlike live messages, which are simply appended, the channel's
    /// `Chat` lines are then rebuilt sorted by `(ts_unix_ms, id)`, since a
    /// backfilled batch can be older than what's already displayed and
    /// mustn't be shown as the newest lines. `System` lines (e.g. earlier
    /// invite output or errors) are left as-is, trailing after the
    /// resorted chat history -- an accepted simplification, since backfill
    /// is expected to land early in a session before much else happens.
    ///
    /// Returns the newly-accepted messages, in canonical order, so the
    /// caller can persist just those to storage.rs without duplicating
    /// anything already on disk.
    pub fn merge_history(&mut self, channel: &str, messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
        let channel_name = channel.to_string();
        let Some(channel) = self.channel_mut(channel) else {
            return Vec::new();
        };
        let mut newly_accepted: Vec<ChatMessage> = messages
            .into_iter()
            .filter(|message| channel.remember_seen(message.id))
            .collect();
        if newly_accepted.is_empty() {
            return newly_accepted;
        }

        let mut chats = Vec::with_capacity(channel.messages.len() + newly_accepted.len());
        let mut systems = Vec::new();
        for line in channel.messages.drain(..) {
            match line {
                TranscriptLine::Chat(message) => chats.push(message),
                TranscriptLine::System(text) => systems.push(text),
            }
        }
        chats.extend(newly_accepted.iter().cloned());
        chats.sort_by_key(|m| (m.ts_unix_ms, m.id));

        for message in chats {
            channel.messages.push_back(TranscriptLine::Chat(message));
        }
        for text in systems {
            channel.messages.push_back(TranscriptLine::System(text));
        }
        while channel.messages.len() > MAX_SCROLLBACK {
            channel.messages.pop_front();
        }

        newly_accepted.sort_by_key(|m| (m.ts_unix_ms, m.id));
        for message in &newly_accepted {
            if let Some(file) = &message.attachment {
                self.record_shared_file(
                    &channel_name,
                    SharedFile {
                        message_id: message.id,
                        file: file.clone(),
                        sender: message.sender,
                        ts_unix_ms: message.ts_unix_ms,
                    },
                );
            }
        }
        newly_accepted
    }

    /// Merges a finished background on-disk scan (`search::run_on_disk_scan`,
    /// kicked off via `InputAction::Search`) into `outcome.channel`'s active
    /// search -- a no-op if that channel isn't currently joined, or if its
    /// active search has since changed (comparing `outcome.term` against
    /// the channel's currently-active one rejects a stale result from a
    /// `/search` that's been superseded or cleared, without needing a
    /// separate request-id).
    ///
    /// Unions rather than replaces the existing snapshot, deduped by id and
    /// re-sorted by `(ts_unix_ms, id)` (mirroring `merge_history`'s
    /// convention above): the on-disk log is a superset of the loaded
    /// scrollback in the overwhelmingly common case, but treating a read
    /// failure (reported as an empty `SearchOutcome`, see
    /// `search::run_on_disk_scan`) as "replace with nothing" would
    /// otherwise erase the in-memory matches already shown.
    pub fn apply_search_outcome(&mut self, outcome: SearchOutcome) {
        let Some(channel) = self.channel_mut(&outcome.channel) else {
            return;
        };
        let Some(search) = &mut channel.search else {
            return;
        };
        if search.term != outcome.term {
            return;
        }
        let mut seen: HashSet<u64> = search.messages.iter().map(|message| message.id).collect();
        for message in outcome.messages {
            if seen.insert(message.id) {
                search.messages.push(message);
            }
        }
        search
            .messages
            .sort_by_key(|message| (message.ts_unix_ms, message.id));
        search.pending = false;
    }

    /// Hands a finished folder read (`picker::read_folder`, kicked off via
    /// `InputAction::ReadFolder`) to the open file picker. A no-op if the
    /// picker has since closed; whether the answer is the one it was
    /// waiting for is the picker's call (`FilePicker::apply`).
    pub fn apply_folder_listing(&mut self, listing: FolderListing) {
        if let Some(picker) = &mut self.picking_file {
            picker.apply(listing);
        }
    }

    pub fn active(&self) -> &Channel {
        &self.channels[self.active]
    }

    pub fn active_mut(&mut self) -> &mut Channel {
        &mut self.channels[self.active]
    }

    fn channel_mut(&mut self, name: &str) -> Option<&mut Channel> {
        self.channels.iter_mut().find(|c| c.name == name)
    }

    /// Clears the newly-active channel's unread state, retaining its oldest
    /// unread message as the renderer's viewport anchor, and clears any
    /// in-progress reply pick or armed reply -- both are only meaningful
    /// against the channel they were made in. The shared tail end of every
    /// place that changes `active`: `switch_channel` (`Tab`/`Shift+Tab`),
    /// `activate_existing_channel`, and `activate_or_create_channel`.
    fn finish_activating_channel(&mut self) {
        let channel = self.active_mut();
        channel.unread_anchor = channel.first_unread.take();
        channel.unread_count = 0;
        self.picking_reply = None;
        self.picking_thread = None;
        self.replying_to = None;
    }

    /// Switches to `name` if it's currently joined, via
    /// `finish_activating_channel`. Returns whether `name` was found --
    /// `/msg`'s reuse path (`run_msg`) uses this to fall back to creating
    /// a brand-new DM channel when the one `crate::dm_registry` remembers
    /// isn't (or is no longer) actually joined.
    fn activate_existing_channel(&mut self, name: &str) -> bool {
        let Some(index) = self.channels.iter().position(|c| c.name == name) else {
            return false;
        };
        self.active = index;
        self.finish_activating_channel();
        true
    }

    /// Switches to `name`, creating a new tab for it first if it isn't
    /// already joined -- shared by `NetEvent::Joined` and
    /// `NetEvent::DmJoined`, both of which report a channel (new or
    /// already-known) finishing its join.
    fn activate_or_create_channel(&mut self, name: String) {
        if !self.activate_existing_channel(&name) {
            self.channels.push(Channel::new(name));
            self.active = self.channels.len() - 1;
            self.finish_activating_channel();
        }
    }

    /// Adds `name` as a new, joined-but-inactive tab -- for a channel
    /// learned via the private device-sync channel (see
    /// `NetEvent::ChannelSyncJoined`), which should appear once actually
    /// joined without yanking focus away from whatever the user is
    /// currently looking at, unlike `activate_or_create_channel`'s
    /// deliberate tab-switch for a user-initiated `/join`. A no-op if
    /// `name` is already a tab (e.g. this device already knew about it
    /// some other way).
    fn add_background_channel(&mut self, name: String) {
        if !self.channels.iter().any(|c| c.name == name) {
            self.channels.push(Channel::new(name));
        }
    }

    /// Records `peer` <-> `channel` in `dm_channels` -- called by main.rs
    /// once a `/msg`-initiated `InputAction::StartDm` actually finishes
    /// joining (a plain `NetEvent::Joined`, correlated back to `peer` via
    /// main.rs's own `pending_dms` map, since a freshly created channel's
    /// `Joined` event has no ticket to carry `peer` through the way
    /// `NetEvent::DmJoined` does for the recipient side -- see that
    /// variant's arm in `handle_net_event`, which updates `dm_channels`
    /// itself instead of needing this).
    pub fn record_dm_channel(&mut self, peer: [u8; 32], channel: String) {
        self.dm_channels.insert(peer, channel);
    }

    /// Classifies `channel` (by its own `name`, the identity key) as a
    /// group channel, a `/msg` DM, or a `/dm` point-to-point chat, and --
    /// for the latter two -- says which peer it's with. The one place that
    /// answers that question: `dm_channels` and `direct_channels` are both
    /// keyed peer -> channel name, so classifying by name is a reverse
    /// lookup. A name present in neither is a group channel, including one
    /// that merely *looks* like a DM (e.g. a user-created `#dm-alice`).
    pub fn channel_kind(&self, channel: &str) -> ChannelKind {
        if let Some(peer) = peer_for_channel(&self.direct_channels, channel) {
            ChannelKind::Direct { peer }
        } else if let Some(peer) = peer_for_channel(&self.dm_channels, channel) {
            ChannelKind::GossipDm { peer }
        } else {
            ChannelKind::Group
        }
    }

    /// The name to *show* a user for `channel`, as opposed to its `name`,
    /// which is its identity (storage file, registry key, gossip map
    /// entry) and must never change. A group channel shows its stored
    /// name, or the local one `/rename` gave it (`renames`). A DM or
    /// point-to-point chat shows whoever it's with, resolved
    /// through `display_name` on every call, so it follows `/alias` and
    /// `/nick` immediately instead of staying frozen at whatever was known
    /// when the channel was created. That frozen name was worse than stale
    /// for the *recipient* of a `/msg` invite: the initiator picks it
    /// (`unique_dm_channel_name`) from their label for the recipient, so on
    /// the recipient's side it described themselves rather than the person
    /// they were talking to.
    ///
    /// Just the name, with no kind marker -- a `/msg` DM and a `/dm` chat
    /// with the same person share one. See `channel_heading` for the form
    /// that says which kind it is.
    pub fn channel_label(&self, channel: &str) -> String {
        match self.channel_kind(channel) {
            ChannelKind::Group => self
                .renames
                .get(channel)
                .cloned()
                .unwrap_or_else(|| channel.to_string()),
            ChannelKind::GossipDm { peer } | ChannelKind::Direct { peer } => {
                self.display_name(&peer)
            }
        }
    }

    /// `channel_label` behind its kind's glyph (`#general`, `@Alice`, a
    /// bolt before `Bob`) -- how a channel is named in pane titles and
    /// system notices, so its kind reads the same everywhere as it does in
    /// the navigation panel, and so two channels that share a label stay
    /// distinguishable.
    pub fn channel_heading(&self, channel: &str) -> String {
        format!(
            "{}{}",
            self.channel_kind(channel).glyph(),
            self.channel_label(channel)
        )
    }

    /// How the navigation panel groups the joined channels: group channels
    /// under one section, DMs and point-to-point chats under the other,
    /// each kept in join order. Every channel lands in exactly one.
    pub fn nav_sections(&self) -> NavSections {
        let mut sections = NavSections {
            channels: Vec::new(),
            people: Vec::new(),
        };
        for (index, channel) in self.channels.iter().enumerate() {
            match self.channel_kind(&channel.name) {
                ChannelKind::Group => sections.channels.push(index),
                ChannelKind::GossipDm { .. } | ChannelKind::Direct { .. } => {
                    sections.people.push(index);
                }
            }
        }
        sections
    }

    /// Every channel index in the order the navigation panel draws them,
    /// top to bottom -- the order `Tab`/`Shift+Tab` step through
    /// (`switch_channel`).
    fn nav_order(&self) -> Vec<usize> {
        let NavSections {
            mut channels,
            people,
        } = self.nav_sections();
        channels.extend(people);
        channels
    }

    /// The peer a point-to-point channel belongs to, if `channel` is one
    /// -- see `channel_kind`. Used by main.rs to decide whether a send (or
    /// a `/leave`) should go through `net::Net::send_direct`/
    /// `close_direct` instead of the gossip-based `send`/`leave`.
    pub fn direct_peer_for(&self, channel: &str) -> Option<[u8; 32]> {
        match self.channel_kind(channel) {
            ChannelKind::Direct { peer } => Some(peer),
            ChannelKind::Group | ChannelKind::GossipDm { .. } => None,
        }
    }

    /// Switches to (creating if necessary) `peer`'s point-to-point
    /// channel, named `channel` (see `direct::channel_name`), and records
    /// the association in `direct_channels`. Returns whether this is a
    /// brand-new tab -- the caller (main.rs) only needs to load persisted
    /// history (`storage::MessageStore`) in that case, mirroring how
    /// startup only loads each gossip channel's history once. Backs the
    /// user-initiated path (`run_dm`'s `InputAction::OpenDirect`): the
    /// local user explicitly asked to go here, so -- like
    /// `activate_or_create_channel` -- this switches focus.
    pub fn activate_direct_channel(&mut self, channel: String, peer: [u8; 32]) -> bool {
        let is_new = !self.channels.iter().any(|c| c.name == channel);
        self.activate_or_create_channel(channel.clone());
        self.direct_channels.insert(peer, channel);
        is_new
    }

    /// Adds `peer`'s point-to-point channel as a background tab without
    /// switching focus, and records the association in `direct_channels`
    /// -- the recipient-side counterpart to `activate_direct_channel`,
    /// for a peer who reached us first with no prior contact on our side
    /// (`net::NetEvent::DirectJoined`). Mirrors `add_background_channel`'s
    /// "don't yank focus" reasoning: the local user never asked for this
    /// conversation to appear. Returns whether this is a brand-new tab,
    /// like `activate_direct_channel`.
    pub fn background_direct_channel(&mut self, channel: String, peer: [u8; 32]) -> bool {
        let is_new = !self.channels.iter().any(|c| c.name == channel);
        self.add_background_channel(channel.clone());
        self.direct_channels.insert(peer, channel);
        is_new
    }

    /// Pure key handling: no I/O, no network. Returns the action to
    /// perform when `Enter` sends a message or runs a `/join`
    /// or `/invite` command, so the caller can talk to the network layer.
    ///
    /// Delegates to `handle_picking_key` first, before any of the normal
    /// typing/editing arms below, whenever a reply pick is in progress
    /// (`picking_reply.is_some()`, see `start_reply_pick`) -- keeps that
    /// transient mode's key handling (`Up`/`Down`/`Enter`/`Esc` all mean
    /// something different while picking) from having to be threaded
    /// through every arm of the match below.
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<InputAction> {
        if self.pending_redeem.is_some() {
            self.handle_redeem_confirmation_key(key);
            return None;
        }
        if self.picking_reply.is_some() {
            self.handle_picking_key(key);
            return None;
        }
        if self.picking_thread.is_some() {
            self.handle_thread_picking_key(key);
            return None;
        }
        if self.switching.is_some() {
            self.handle_switcher_key(key);
            return None;
        }
        if self.command_picker.is_some() {
            return self.handle_command_picker_key(key);
        }
        if self.viewing_peers.is_some() {
            self.handle_peers_overlay_key(key);
            return None;
        }
        if self.picking_file.is_some() {
            return self.handle_picker_key(key);
        }
        if self.viewing_files.is_some() {
            return self.handle_files_key(key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            // Backs out one level at a time: cancel an armed reply first
            // (mirroring `handle_picking_key`'s own Esc, which cancels a
            // pick-in-progress instead of confirming it), then clear an
            // active search, drop any staged attachments, leave an open
            // thread view, only quitting once none of those is active.
            KeyCode::Esc => {
                if self.replying_to.take().is_some() {
                    // cancelled the armed reply, nothing further to do
                } else if self.clear_active_search() {
                    // left the filtered transcript, nothing further to do
                } else if self.clear_staged() {
                    // dropped the staged attachments, nothing further to do
                } else if !self.close_thread() {
                    self.should_quit = true;
                }
            }
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Char('r') if ctrl => self.start_reply_pick(),
            KeyCode::Char('t') if ctrl => self.start_thread_pick(),
            KeyCode::Char('p') if ctrl => self.start_switcher(),
            KeyCode::Char('a') if ctrl => self.active_mut().move_home(),
            KeyCode::Char('e') if ctrl => self.active_mut().move_end(),
            KeyCode::Char('u') if ctrl => self.active_mut().delete_to_start(),
            KeyCode::Char('k') if ctrl => self.start_command_picker(),
            KeyCode::Char('w') if ctrl => self.active_mut().delete_word_backward(),
            // Inserts a literal newline instead of submitting. Ctrl+J is
            // the plain ASCII linefeed byte, so every terminal reports it
            // the same way regardless of Alt/Meta configuration. Alt+Enter
            // is the more discoverable binding (mirrors Slack/Discord's
            // Shift+Enter convention) and works just as reliably: any
            // terminal using the standard xterm Alt-prefixing convention
            // sends `ESC` then `\r`, which crossterm reports as `Enter`
            // with the `ALT` modifier. Both are checked before the plain
            // `Enter` arm below so a bare `Enter` still always submits.
            KeyCode::Char('j') if ctrl => self.active_mut().insert_newline(),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => {
                self.active_mut().insert_newline()
            }
            // Cmd on macOS -- reported as `KeyModifiers::SUPER` by
            // crossterm, if the terminal forwards it at all. Plain Ctrl+V
            // is the dependable cross-platform trigger, since terminals
            // (Warp included) conventionally reserve their own paste
            // shortcut as Ctrl+Shift+V/Cmd+V and consume it before it ever
            // reaches a foreground app -- see `run_paste`'s doc comment.
            KeyCode::Char('v') if ctrl || key.modifiers.contains(KeyModifiers::SUPER) => {
                return self.run_paste();
            }
            KeyCode::Enter => return self.submit_input(),
            // With an empty input there is no character to delete, so the
            // key takes back the newest staged attachment instead -- like
            // removing the last chip from an address field.
            KeyCode::Backspace => {
                let channel = self.active_mut();
                if channel.input.is_empty() && !channel.staged.is_empty() {
                    channel.staged.pop();
                } else {
                    channel.delete_backward();
                }
            }
            KeyCode::Delete => self.active_mut().delete_forward(),
            KeyCode::Home => self.active_mut().move_home(),
            KeyCode::End => self.active_mut().move_end(),
            KeyCode::Left => self.active_mut().move_left(),
            KeyCode::Right => self.active_mut().move_right(),
            KeyCode::Up => {
                let channel = self.active_mut();
                channel.unread_anchor = None;
                channel.scroll = channel.scroll.saturating_add(1);
            }
            KeyCode::Down => {
                let channel = self.active_mut();
                channel.unread_anchor = None;
                channel.scroll = channel.scroll.saturating_sub(1);
            }
            KeyCode::Tab => self.switch_channel(1),
            KeyCode::BackTab => self.switch_channel(-1),
            KeyCode::Char(c) => self.active_mut().insert_char(c),
            _ => {}
        }
        None
    }

    /// Handles a keypress while confirming a live pairing redeem (`/pair
    /// <ticket>`, see `run_pair`): `Enter` confirms -- taking
    /// `pending_redeem` into `restart_with_ticket` and setting
    /// `should_quit`, so main.rs's normal quit path (loop exit, `Net::
    /// shutdown`, terminal restore) runs first, then wipes local state and
    /// restarts into `--pair <ticket>` -- `Esc` cancels back to normal
    /// typing. `Ctrl+C` still force-quits (a plain, non-pairing quit,
    /// `restart_with_ticket` stays unset), mirroring `handle_picking_key`.
    /// Every other key is ignored: confirming this is destructive enough
    /// to warrant an explicit, narrow yes/no gate rather than letting a
    /// stray keystroke do anything else.
    fn handle_redeem_confirmation_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }
        match key.code {
            KeyCode::Enter => {
                if let Some(ticket) = self.pending_redeem.take() {
                    self.restart_with_ticket = Some(ticket);
                    self.should_quit = true;
                }
            }
            KeyCode::Esc => {
                self.pending_redeem = None;
                self.push_system("pairing cancelled");
            }
            _ => {}
        }
    }

    /// Handles a keypress while picking a reply target (`Ctrl+R`, see
    /// `start_reply_pick`): `Up`/`Down` move the highlighted candidate
    /// (`move_reply_pick`), `Enter` arms it as `replying_to`, `Esc`
    /// cancels back to normal typing. `Ctrl+C` still force-quits, since
    /// `Esc` no longer does while picking. Every other key is ignored --
    /// picking is a dedicated mode, not an overlay on normal editing, so a
    /// stray keystroke can't leak into the input box.
    fn handle_picking_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }
        match key.code {
            KeyCode::Up => self.move_reply_pick(-1),
            KeyCode::Down => self.move_reply_pick(1),
            KeyCode::Enter => self.replying_to = self.picking_reply.take(),
            KeyCode::Esc => self.picking_reply = None,
            _ => {}
        }
    }

    /// Handles a keypress while picking a message to view its thread
    /// (`Ctrl+T`, see `start_thread_pick`): `Up`/`Down` move the
    /// highlighted candidate (`move_thread_pick`), `Enter` opens its
    /// thread view (`open_thread`), `Esc` cancels back to normal typing.
    /// Mirrors `handle_picking_key`'s shape, but for thread-picking
    /// instead of reply-picking -- kept as a separate method (rather than
    /// a shared one parameterized by purpose) since the two confirm
    /// actions differ.
    fn handle_thread_picking_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }
        match key.code {
            KeyCode::Up => self.move_thread_pick(-1),
            KeyCode::Down => self.move_thread_pick(1),
            KeyCode::Enter => {
                if let Some(id) = self.picking_thread.take() {
                    self.open_thread(id);
                }
            }
            KeyCode::Esc => self.picking_thread = None,
            _ => {}
        }
    }

    /// Handles `Ctrl+P`: opens the jump-to-conversation overlay with an
    /// empty query, listing every channel -- type to filter, move with
    /// `Up`/`Down`, confirm with `Enter`, cancel with `Esc` (see
    /// `handle_switcher_key`). `Ctrl+K`, the usual quick-switch key, is
    /// already `delete_to_end`, so this deliberately takes `Ctrl+P`.
    fn start_switcher(&mut self) {
        self.switching = Some(Switcher::default());
    }

    /// Handles a keypress while the switcher is open: typing and
    /// `Backspace` edit the query, `Up`/`Down` move the highlight, `Enter`
    /// switches to it (`confirm_switcher`), `Esc` closes without
    /// switching. `Ctrl+C` still force-quits, since `Esc` no longer does
    /// while the overlay is open. Every other key is ignored -- like the
    /// pick modes above this is a dedicated mode, so a stray keystroke
    /// can't leak into the input box.
    fn handle_switcher_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Esc => self.switching = None,
            KeyCode::Enter => self.confirm_switcher(),
            KeyCode::Up => self.move_switcher(-1),
            KeyCode::Down => self.move_switcher(1),
            KeyCode::Backspace => self.edit_switcher_query(|query| {
                query.pop();
            }),
            KeyCode::Char(c) if !ctrl => self.edit_switcher_query(|query| query.push(c)),
            _ => {}
        }
    }

    /// Applies `edit` to the open switcher's query and puts the highlight
    /// back on the first match. The one place the query changes, so a typed
    /// character, a `Backspace` and a paste can't disagree about where the
    /// highlight lands. A no-op if the switcher isn't open.
    fn edit_switcher_query(&mut self, edit: impl FnOnce(&mut String)) {
        if let Some(switcher) = &mut self.switching {
            edit(&mut switcher.query);
            switcher.selected = 0;
        }
    }

    /// The channels the open switcher lists, as indices into `channels`:
    /// those whose label (`channel_label` -- what's on screen, never the
    /// stored name) contains the typed text, ignoring case, in the order
    /// the navigation panel draws them (`nav_order`), so the list reads
    /// like the panel and steps like `Tab`. An empty query lists every
    /// channel; nothing is listed while the switcher is closed.
    pub fn switcher_matches(&self) -> Vec<usize> {
        let Some(switcher) = &self.switching else {
            return Vec::new();
        };
        let query = switcher.query.to_lowercase();
        self.nav_order()
            .into_iter()
            .filter(|&index| {
                self.channel_label(&self.channels[index].name)
                    .to_lowercase()
                    .contains(&query)
            })
            .collect()
    }

    /// The channel (an index into `channels`) `Enter` would switch to: the
    /// highlighted match, or `None` while nothing matches or the switcher
    /// is closed. The stored position is clamped here, and only here --
    /// matches can change under an open overlay (a peer gets renamed, a
    /// channel joins), and a position left over from before must never
    /// point past the end.
    pub fn switcher_highlight(&self) -> Option<usize> {
        let switcher = self.switching.as_ref()?;
        let matches = self.switcher_matches();
        let last = matches.len().checked_sub(1)?;
        Some(matches[switcher.selected.min(last)])
    }

    /// Moves the highlight by `delta` among the current matches, clamping
    /// at both ends rather than wrapping, like `move_pick` does for
    /// messages. A no-op while nothing matches.
    fn move_switcher(&mut self, delta: isize) {
        let Some(last) = self.switcher_matches().len().checked_sub(1) else {
            return;
        };
        if let Some(switcher) = &mut self.switching {
            let current = switcher.selected.min(last) as isize;
            switcher.selected = (current + delta).clamp(0, last as isize) as usize;
        }
    }

    /// Switches to the highlighted match and closes the switcher. Does
    /// nothing while nothing matches, leaving the overlay open to edit.
    /// Choosing the channel you're already on just closes it: a real switch
    /// clears an armed reply (`finish_activating_channel`), and staying put
    /// mustn't discard one for no reason.
    fn confirm_switcher(&mut self) {
        let Some(index) = self.switcher_highlight() else {
            return;
        };
        self.switching = None;
        if index != self.active {
            self.active = index;
            self.finish_activating_channel();
        }
    }

    fn start_command_picker(&mut self) {
        self.command_picker = Some(CommandPicker::default());
    }

    fn handle_command_picker_key(&mut self, key: KeyEvent) -> Option<InputAction> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if ctrl => self.should_quit = true,
            KeyCode::Esc => self.command_picker = None,
            KeyCode::Enter => return self.confirm_command_picker(),
            KeyCode::Up => self.move_command_picker(-1),
            KeyCode::Down => self.move_command_picker(1),
            KeyCode::Backspace => self.edit_command_picker_query(|query| {
                query.pop();
            }),
            KeyCode::Char(c) if !ctrl => self.edit_command_picker_query(|query| query.push(c)),
            _ => {}
        }
        None
    }

    fn handle_peers_overlay_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
        } else if key.code == KeyCode::Esc {
            self.viewing_peers = None;
        }
    }

    fn edit_command_picker_query(&mut self, edit: impl FnOnce(&mut String)) {
        if let Some(picker) = &mut self.command_picker {
            edit(&mut picker.query);
            picker.selected = 0;
        }
    }

    pub fn command_picker_matches(&self) -> Vec<usize> {
        let Some(picker) = &self.command_picker else {
            return Vec::new();
        };
        let query = picker.query.to_lowercase();
        let command_name_matches = command_catalog()
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                command.syntax.to_lowercase().contains(&query)
                    || command
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase().contains(&query))
            })
            .map(|(index, _)| index);
        let description_matches = command_catalog()
            .iter()
            .enumerate()
            .filter(|(_, command)| {
                !command.syntax.to_lowercase().contains(&query)
                    && !command
                        .aliases
                        .iter()
                        .any(|alias| alias.to_lowercase().contains(&query))
                    && command.description.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index);
        command_name_matches.chain(description_matches).collect()
    }

    pub fn command_picker_highlight(&self) -> Option<usize> {
        let picker = self.command_picker.as_ref()?;
        let matches = self.command_picker_matches();
        let last = matches.len().checked_sub(1)?;
        Some(matches[picker.selected.min(last)])
    }

    fn move_command_picker(&mut self, delta: isize) {
        let Some(last) = self.command_picker_matches().len().checked_sub(1) else {
            return;
        };
        if let Some(picker) = &mut self.command_picker {
            let current = picker.selected.min(last) as isize;
            picker.selected = (current + delta).clamp(0, last as isize) as usize;
        }
    }

    fn confirm_command_picker(&mut self) -> Option<InputAction> {
        let Some(index) = self.command_picker_highlight() else {
            return None;
        };
        let command = command_catalog()[index]
            .input
            .trim_start_matches('/')
            .trim();
        self.command_picker = None;
        self.run_command(command)
    }

    /// Opens the file picker -- a bare `/send` -- for the channel that is
    /// active now, in the folder files were last chosen from, and asks the
    /// caller to read it. The picker shows "reading" until the listing
    /// arrives (`apply_folder_listing`), so input is never held up on it.
    fn open_picker(&mut self) -> InputAction {
        let dir = self.browse_dir.clone();
        let request = next_request(&mut self.browse_requests);
        self.picking_file = Some(FilePicker::new(
            self.active().name.clone(),
            dir.clone(),
            request,
        ));
        InputAction::ReadFolder { request, dir }
    }

    /// Handles a keypress while the file picker is open: what the keys
    /// mean is `FilePicker::handle_key`'s business, and this turns what it
    /// asks for into state changes and an `InputAction`. `Ctrl+C` still
    /// force-quits, since `Esc` only closes the picker.
    fn handle_picker_key(&mut self, key: KeyEvent) -> Option<InputAction> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return None;
        }
        let picker = self
            .picking_file
            .as_mut()
            .expect("handle_key only calls this while the picker is open");
        match picker.handle_key(key) {
            Step::Stay => None,
            Step::Cancel => {
                self.picking_file = None;
                None
            }
            Step::Open(dir) => {
                let request = next_request(&mut self.browse_requests);
                picker.read(request, dir.clone());
                Some(InputAction::ReadFolder { request, dir })
            }
            Step::Choose(paths) => {
                let picker = self.picking_file.take()?;
                self.browse_dir = picker.dir().to_path_buf();
                Some(InputAction::StageFiles {
                    channel: picker.channel,
                    paths,
                    folder: self.browse_dir.clone(),
                })
            }
        }
    }

    /// Handles `/files`: opens the files overlay on the active channel with
    /// an empty query, listing every file shared there -- type to filter,
    /// move with `Up`/`Down`, `Enter` saves, `Ctrl+O` opens one already
    /// saved, `Esc` closes (see `handle_files_key`). Which of them are saved
    /// is a fact about the disk, so the caller is asked
    /// (`InputAction::CheckSavedFiles`) and answers at once
    /// Applies a network event: updates presence, pushes a deduped incoming
    /// message, records a peer's broadcast nickname, or reports a channel
    /// join's outcome. Returns the channel name and message when `event`
    /// was a `NetEvent::Received` that resulted in a newly-accepted
    /// (non-duplicate) message, so the caller can persist it (see
    /// storage.rs) -- `None` for every other event, including a duplicate
    /// Removes `channel`'s tab and all its in-memory state (transcript,
    /// presence, dedupe buffers, any DM or point-to-point association, any
    /// `/rename` display name, and its auto-save setting -- left behind, the
    /// first would misclassify, the second mislabel and the third switch
    /// downloads on in a later channel that happens to reuse the name, see
    /// `channel_kind`/`channel_label`/`autosave`) -- called once the
    /// caller has actually torn down its gossip subscription and persisted
    /// state (see `InputAction::Leave`). Adjusts `active` to keep pointing
    /// at the same tab if it was after the removed one, or lands on the
    /// previous tab if the active tab itself was the one removed. A
    /// no-op if `channel` isn't currently joined.
    ///
    /// # Panics
    /// Panics if `channel` is leyline's only joined channel -- `AppState`
    /// always needs at least one (see `AppState::new`), and `run_leave`
    /// is the only caller that decides whether to request a removal, so
    /// it must already refuse this case before ever returning
    /// `InputAction::Leave`.
    pub fn remove_channel(&mut self, channel: &str) {
        let Some(index) = self.channels.iter().position(|c| c.name == channel) else {
            return;
        };
        assert!(
            self.channels.len() > 1,
            "must not remove leyline's only channel"
        );
        // Resolved before the association and rename are dropped below:
        // once they're gone the channel would fall back to showing its
        // raw name, and the notice should say what the channel was
        // actually called on screen.
        let heading = self.channel_heading(channel);
        self.dm_channels.retain(|_, name| name != channel);
        self.direct_channels.retain(|_, name| name != channel);
        self.direct_folders.remove(channel);
        self.shared_files.remove(channel);
        self.renames.remove(channel);
        self.autosave.remove(channel);
        self.autosave_limits.remove(channel);
        self.outbox.remove(channel);
        self.auto_saving.retain(|(name, _)| name != channel);
        self.channels.remove(index);
        if self.active > index {
            self.active -= 1;
        } else if self.active == index {
            self.active = self.active.min(self.channels.len().saturating_sub(1));
            // Only this branch actually switches to a different channel
            // (the other one just re-numbers the same channel after an
            // earlier tab's removal) -- a reply id only makes sense
            // against the channel it was picked in.
            self.picking_reply = None;
            self.picking_thread = None;
            self.replying_to = None;
        }
        self.push_system(format!("left {heading}"));
    }

    /// Places a locally-authored message (currently only a finished
    /// `/send`, via `NetEvent::FileReady`) into `channel`'s transcript,
    /// deduped the same way an incoming message is -- called by main.rs
    /// once it has broadcast and persisted the message, since its content
    /// (the file's hash) isn't known until the background import
    /// finishes. Unlike `handle_net_event`'s `Received` case, never marks
    /// the channel unread -- there's no point notifying yourself about
    /// your own successfully sent file. A no-op if `channel` isn't
    /// currently joined.
    pub fn record_sent_message(&mut self, channel: &str, message: ChatMessage) {
        if let Some(file) = &message.attachment {
            self.record_shared_file(
                channel,
                SharedFile {
                    message_id: message.id,
                    file: file.clone(),
                    sender: message.sender,
                    ts_unix_ms: message.ts_unix_ms,
                },
            );
        }
        if let Some(channel) = self.channel_mut(channel)
            && channel.remember_seen(message.id)
        {
            channel.push(TranscriptLine::Chat(message));
        }
    }

    /// Stages `item` -- already vetted by `files::stage_file`/
    /// `stage_image` -- on `channel`, ready for review and `Enter`. Called
    /// by main.rs when a clipboard read (`net::NetEvent::ClipboardFiles`/
    /// `ClipboardImage`) comes back, which may be after the user has moved
    /// to another tab; the event names the channel the paste was for, so
    /// the attachment lands there rather than wherever focus is now. A
    /// no-op if `channel` isn't currently joined.
    pub fn stage_attachment(&mut self, channel: &str, item: StagedAttachment) {
        if let Some(channel) = self.channel_mut(channel) {
            channel.staged.push(item);
        }
    }

    /// Composes a `ChatMessage` as if `text` had been typed and sent by us
    /// -- used for `/paste`'s clipboard-text fallback (see
    /// `net::NetEvent::ClipboardText`), which arrives asynchronously from
    /// the background clipboard read rather than through
    /// `handle_key`/`submit_input`. The caller still needs to broadcast,
    /// persist, and display the result exactly like any other
    /// locally-authored message (see `record_sent_message`). Never a
    /// reply -- this flow doesn't go through the reply-picking UI.
    pub fn compose_own_message(&self, text: &str) -> ChatMessage {
        compose_message(self.self_id, text, None)
    }

    /// Displays a sender as "you" for our own id, or for *any* peer
    /// certified under our own `self_user_id` -- i.e. one of our own other
    /// devices, paired via `/pair` (see `canonical_id`) -- since from a
    /// human's perspective a message or presence from your other device is
    /// still just you, not a second stranger; their pet name if one has
    /// been assigned via `/alias` (keyed by their resolved `canonical_id`,
    /// not necessarily the literal `sender`, so an alias set against any
    /// one of a person's devices applies to all of them once their
    /// certificates are known -- see `canonical_id`); else their last
    /// broadcast nickname from `/nick`, suffixed with their hex prefix
    /// since a nickname is spoofable and not unique (features.md's
    /// "Broadcast nicknames"); or, absent both, a shortened hex id on its
    /// own.
    pub fn display_name(&self, sender: &[u8; 32]) -> String {
        if *sender == self.self_id {
            return "you".to_string();
        }
        let id = self.canonical_id(sender);
        if id == self.self_user_id {
            return "you".to_string();
        }
        if let Some(name) = self.petnames.get(&id) {
            name.clone()
        } else if let Some(nickname) = self.nicknames.get(&id) {
            format!("{nickname} ({})", hex_prefix(sender))
        } else {
            hex_prefix(sender)
        }
    }

    /// Resolves `id` to the user identity it belongs to, if a verified
    /// `crate::message::DeviceCert` has been seen for it (`device_to_user`)
    /// -- otherwise `id` unchanged, which is also correct for our own id
    /// and for any peer we have no cert for (an older build, or one not
    /// yet verified). `display_name`, `/who`'s grouping
    /// (`grouped_who_lines`), and `/alias` (`run_alias`) all resolve
    /// through this first, so two of one person's devices collapse onto
    /// whichever single petname/nickname has been recorded for that
    /// person, instead of needing one recorded per device. Note that this
    /// never maps our own `self_id` to `self_user_id` -- nothing ever
    /// sends us a certificate for our own device id, since `announce_
    /// device` only ever broadcasts outward -- so callers that need to
    /// recognize *our own* sibling devices (`display_name`, `unique_
    /// peer_ids`) compare a peer's `canonical_id` against `self_user_id`
    /// directly instead of relying on this method alone.
    ///
    /// A forgotten device (`/forget-device`, see `forgotten`) is always
    /// treated as if no certificate were known for it, regardless of what
    /// `device_to_user` still remembers -- forgetting takes effect
    /// immediately, not just for future certificates.
    pub fn canonical_id(&self, id: &[u8; 32]) -> [u8; 32] {
        if self.forgotten.contains(id) {
            return *id;
        }
        self.device_to_user.get(id).copied().unwrap_or(*id)
    }

    /// Deduplicates `ids` by `canonical_id`, keeping the first device seen
    /// for each distinct person and preserving their original relative
    /// order -- the same grouping `grouped_who_lines` applies for `/who`,
    /// just without that command's "(N devices)" breakdown text, which
    /// stays exclusively `/who`'s. Also excludes any peer certified under
    /// our own `self_user_id` entirely, rather than merging it into some
    /// other group: one of our own other devices is already represented
    /// by the sidebar's separate "you" row (`ui::render_peers`), so
    /// listing it again here would just be the same person shown twice
    /// under two different labels. A peer with no known certificate is
    /// simply its own one-member group, so this changes nothing for it.
    #[cfg(test)]
    pub fn unique_peer_ids(&self, ids: &[[u8; 32]]) -> Vec<[u8; 32]> {
        let mut seen = HashSet::new();
        ids.iter()
            .filter(|id| self.canonical_id(id) != self.self_user_id)
            .filter(|id| seen.insert(self.canonical_id(id)))
            .copied()
            .collect()
    }

    /// The active channel's peers, with currently-online people first and
    /// remembered offline people after them. Multiple endpoint ids certified
    /// for one person collapse to one row; live presence wins over a
    /// remembered sibling device, and otherwise the newest observation wins.
    pub fn peer_presence(&self) -> Vec<PeerPresence> {
        let channel = self.active();
        let mut peers = Vec::new();

        for id in &channel.peers {
            let canonical = self.canonical_id(id);
            if canonical == self.self_user_id {
                continue;
            }
            if peers
                .iter()
                .any(|peer: &PeerPresence| self.canonical_id(&peer.id) == canonical)
            {
                continue;
            }
            peers.push(PeerPresence {
                id: *id,
                last_seen_unix_ms: None,
            });
        }

        let mut offline: Vec<_> = channel.last_seen.iter().collect();
        offline.sort_by_key(|(id, seen)| (std::cmp::Reverse(**seen), **id));
        for (id, seen_unix_ms) in offline {
            let canonical = self.canonical_id(id);
            if canonical == self.self_user_id {
                continue;
            }
            match peers
                .iter_mut()
                .find(|peer| self.canonical_id(&peer.id) == canonical)
            {
                Some(peer) if peer.last_seen_unix_ms.is_none() => {}
                Some(peer) if peer.last_seen_unix_ms < Some(*seen_unix_ms) => {
                    *peer = PeerPresence {
                        id: *id,
                        last_seen_unix_ms: Some(*seen_unix_ms),
                    };
                }
                Some(_) => {}
                None => peers.push(PeerPresence {
                    id: *id,
                    last_seen_unix_ms: Some(*seen_unix_ms),
                }),
            }
        }

        peers
    }

    /// The newest peer version we've heard (via `NetEvent::Version`) that's
    /// actually newer than ours, if any -- used by `ui::render_header`'s
    /// persistent indicator and `/version`'s own report. Recomputed on
    /// demand rather than cached, since `peer_versions` is small (bounded
    /// by distinct peers ever seen, like `nicknames`).
    pub fn newest_known_update(&self) -> Option<(&str, &str)> {
        self.peer_versions
            .values()
            .filter(|(candidate, _)| version::is_newer(candidate))
            .max_by_key(|(candidate, _)| version::parse(candidate))
            .map(|(candidate, hash)| (candidate.as_str(), hash.as_str()))
    }

    /// Appends a local-only system notice to the active channel's
    /// transcript (invite output, command errors) -- never sent over the
    /// network.
    pub fn push_system(&mut self, text: impl Into<String>) {
        self.active_mut().push(TranscriptLine::System(text.into()));
    }

    /// Appends a local-only system notice to `channel`'s transcript, wherever
    /// the user happens to be looking -- for what concerns one channel and
    /// happens without anyone having asked just now (auto-save). A no-op if
    /// `channel` isn't joined, which is only the case once it has been left.
    pub fn push_system_to(&mut self, channel: &str, text: impl Into<String>) {
        if let Some(channel) = self.channel_mut(channel) {
            channel.push(TranscriptLine::System(text.into()));
        }
    }

    /// Says that a save is over, and how it went (`notice`), where its `mode`
    /// puts it: a save someone asked for is answered in the channel they are
    /// in, as it always was; an automatic one in the channel its file arrived
    /// in -- and is no longer in flight, so the same file can be saved again
    /// (`auto_saving`).
    fn finish_save(&mut self, channel: &str, hash: Hash, mode: SaveMode, notice: String) {
        match mode {
            SaveMode::Manual => self.push_system(notice),
            SaveMode::Automatic { .. } => {
                self.auto_saving.remove(&(channel.to_string(), hash));
                self.push_system_to(channel, notice);
            }
        }
    }

    /// Says that a save didn't happen -- the download failed
    /// (`NetEvent::FileSaveFailed`) or there was nowhere to put the file
    /// (main.rs's `start_save`) -- `error` being the reason. A save someone
    /// asked for says so exactly as it always did; an automatic one says
    /// which file and how to fetch it by hand, in the channel it arrived in.
    pub fn save_failed(
        &mut self,
        channel: &str,
        hash: Hash,
        filename: &str,
        error: &str,
        mode: SaveMode,
    ) {
        let notice = match mode {
            SaveMode::Manual => format!("failed to save {filename}: {error}"),
            SaveMode::Automatic { .. } => format!(
                "couldn't auto-save {filename}: {error} -- {} to download it yourself",
                save_hint(hash)
            ),
        };
        self.finish_save(channel, hash, mode, notice);
    }

    /// Moves `active` forward (`step = 1`) or backward (`step = -1`)
    /// through the channels in the order the navigation panel draws them
    /// (`nav_order`), wrapping, and clears the new active channel's unread
    /// state. Stepping in drawn order rather than join order keeps the
    /// panel's highlight moving top to bottom instead of hopping between
    /// its sections when group channels and conversations were joined
    /// interleaved; with only group channels the two orders are the same.
    /// Also clears any in-progress reply pick or armed reply -- both are
    /// only meaningful against the channel they were made in.
    fn switch_channel(&mut self, step: isize) {
        let order = self.nav_order();
        if order.len() <= 1 {
            return;
        }
        let position = order
            .iter()
            .position(|&index| index == self.active)
            .expect("the active channel is always in the navigation order");
        let next = (position as isize + step).rem_euclid(order.len() as isize) as usize;
        self.active = order[next];
        self.finish_activating_channel();
    }

    /// `Enter`: runs a `/command`, or sends what was typed -- as a plain
    /// message or, with attachments staged on the active channel
    /// (`Channel::staged`), as the caption of the first of them. A
    /// `/command` runs as one even then (the stage is left alone), so a
    /// command can't be swallowed as a caption; and with nothing typed,
    /// staged attachments still go out on their own.
    fn submit_input(&mut self) -> Option<InputAction> {
        let channel = self.active_mut();
        if channel.input.trim().is_empty() && channel.staged.is_empty() {
            return None;
        }
        let text = std::mem::take(&mut channel.input);
        channel.cursor = 0;
        channel.scroll = 0;

        if let Some(rest) = text.strip_prefix('/') {
            return self.run_command(rest.trim());
        }

        if !self.active().staged.is_empty() {
            return Some(self.send_staged(text));
        }

        let reply_to = self
            .replying_to
            .take()
            .or_else(|| self.active_thread_reply_target());
        Some(self.send_text(&text, reply_to))
    }

    /// The reply target a plain, non-explicitly-armed message should
    /// automatically attach to: the newest message in the active
    /// channel's open `/thread` view (`origin` itself, until the thread
    /// gains descendants of its own), or `None` if no thread view is
    /// open. Without this, a message sent while reading a thread would
    /// land with no `reply_to` at all -- part of the active channel, but
    /// not part of the conversation being viewed. Only consulted by
    /// `submit_input` after `replying_to` (armed via `Ctrl+R`/`/reply`),
    /// so an explicit pick always takes precedence over this default.
    fn active_thread_reply_target(&self) -> Option<u64> {
        self.active()
            .thread
            .as_ref()?
            .messages
            .last()
            .map(|message| message.id)
    }

    /// Composes `text` as a message from us -- replying to `reply_to` if
    /// given -- records it in the active channel's transcript, and
    /// returns the `InputAction` for the caller to broadcast/persist. The
    /// shared tail end of sending a plain typed message (`submit_input`,
    /// which passes the armed `replying_to` if any) and `/reply <text>`'s
    /// one-shot send (`run_reply`).
    fn send_text(&mut self, text: &str, reply_to: Option<u64>) -> InputAction {
        let channel = self.active().name.clone();
        let message = compose_message(self.self_id, text, reply_to);
        self.active_mut().remember_seen(message.id);
        self.active_mut()
            .push(TranscriptLine::Chat(message.clone()));
        InputAction::Send(channel, message)
    }

    /// Sends everything staged on the active channel: takes the stage (so
    /// it is empty from here on), leaves a breadcrumb per file the way
    /// `run_send` does -- importing and announcing happen in the
    /// background and may take a moment -- and returns the `InputAction`
    /// for the caller to act on. `text` is whatever was typed; unless it is
    /// only whitespace it becomes the caption of the first attachment. An
    /// armed reply or open thread is left as it was: a shared file is
    /// never a reply, same as `/send` today.
    fn send_staged(&mut self, text: String) -> InputAction {
        let channel = self.active().name.clone();
        let items = std::mem::take(&mut self.active_mut().staged);
        for item in &items {
            self.push_system(format!("sending {}...", item.filename));
        }
        let caption = if text.trim().is_empty() {
            String::new()
        } else {
            text
        };
        InputAction::SendStaged {
            channel,
            items,
            caption,
        }
    }

    /// Drops everything staged on the active channel -- `Esc`'s second
    /// level, see `handle_key`. Returns whether there was anything to drop,
    /// so the caller knows whether the key was used up.
    fn clear_staged(&mut self) -> bool {
        let staged = &mut self.active_mut().staged;
        let had_any = !staged.is_empty();
        staged.clear();
        had_any
    }

    /// Clears the active channel's search filter and records the result.
    /// Returns whether a filter was active, so callers such as `Esc` know
    /// whether they consumed the key.
    fn clear_active_search(&mut self) -> bool {
        let had_search = self.active_mut().search.take().is_some();
        if had_search {
            self.push_system("search cleared");
        }
        had_search
    }

    /// Parses a `/command [args]` line (the text after the leading `/`,
    /// already trimmed). Recognized commands may still return `None` after
    /// pushing a system notice (e.g. usage errors) -- only `/join`,
    /// `/invite`, and a successful `/alias` or `/nick` need the caller to
    /// actually do anything.
    fn run_command(&mut self, command: &str) -> Option<InputAction> {
        let (name, arg) = command.split_once(' ').unwrap_or((command, ""));
        match name {
            "join" => {
                let arg = arg.trim();
                if arg.is_empty() {
                    self.push_system("usage: /join <channel-name-or-ticket>");
                    None
                } else if arg == DEVICE_SYNC_CHANNEL {
                    self.push_system(format!(
                        "#{DEVICE_SYNC_CHANNEL} is reserved for syncing your own paired devices"
                    ));
                    None
                } else {
                    Some(InputAction::Join(arg.to_string()))
                }
            }
            "invite" => self.run_invite(),
            "pair" => self.run_pair(arg.trim()),
            "forget-device" => self.run_forget_device(arg.trim()),
            "msg" => self.run_msg(arg.trim()),
            "dm" => self.run_dm(arg.trim()),
            "alias" => self.run_alias(arg.trim()),
            "nick" => self.run_nick(arg.trim()),
            "leave" => self.run_leave(arg.trim()),
            "rename" => self.run_rename(arg.trim()),
            "who" => self.run_who(),
            "send" => self.run_send(arg.trim()),
            "save" => self.run_save(arg.trim()),
            "folder" => self.run_folder(arg.trim()),
            "maxsize" => self.run_maxsize(arg.trim()),
            "autosave" => self.run_autosave(arg.trim()),
            "outbox" => self.run_outbox(arg.trim()),
            "sync" => self.run_sync(arg.trim()),
            "activity" => Some(InputAction::ListSyncActivity(self.active().name.clone())),
            "conflicts" => self.run_conflicts(arg.trim()),
            "trash" => self.run_trash(arg.trim()),
            "files" => self.run_files(),
            "paste" => self.run_paste(),
            "search" | "s" => self.run_search(arg.trim()),
            "reply" => self.run_reply(arg.trim()),
            "thread" => self.run_thread(),
            "hints" => self.run_hints(),
            "bell" => self.run_bell(),
            "version" => self.run_version(),
            "help" => self.run_help(),
            _ => {
                self.push_system(format!("unknown command: /{name}"));
                None
            }
        }
    }

    /// Handles `/invite`: builds an invite ticket for the active channel,
    /// unless it's a point-to-point one (see `direct_channels`) -- those
    /// have no ticket or room secret at all (`net::Net::ticket_for` would
    /// panic if asked for one), so this refuses with a clear message
    /// instead of ever reaching the caller's network layer.
    fn run_invite(&mut self) -> Option<InputAction> {
        let channel = self.active().name.clone();
        if self.direct_peer_for(&channel).is_some() {
            self.push_system(
                "point-to-point conversations have no invite ticket -- share your endpoint id directly instead",
            );
            return None;
        }
        Some(InputAction::Invite(channel))
    }

    /// Handles `/help`: pushes the full command/keybinding reference as a
    /// system notice. The input box's border only shows a short hint (see
    /// `ui::render_input`) to stay uncluttered, so this is where that
    /// detail actually lives. One entry per line, with a blank line between
    /// entries -- `ui::build_rows` renders each `\n` as its own row.
    fn run_help(&mut self) -> Option<InputAction> {
        let autosave_limit = human_size(self.autosave_limit.max_bytes());
        let commands = [
            "/join <name|ticket> -- join a channel from a ticket, or create a new private one by name".to_string(),
            "/invite -- show an invite ticket for this channel".to_string(),
            "/pair [ticket] -- with no argument, shows a ticket for a new device to redeem; /pair <ticket> redeems one (after an Enter/Esc confirmation -- this replaces this device's identity and data)".to_string(),
            "/forget-device <hex-prefix> -- forget a paired device".to_string(),
            "/msg <alias|hex-prefix> [text] -- start a private channel message with a peer".to_string(),
            "/dm <alias|hex-prefix|endpoint-id> [text] -- connect directly to a peer's endpoint id with no ticket or gossip channel, unlike /msg".to_string(),
            "/leave [channel] -- leave a channel".to_string(),
            "/rename [name] -- set this channel's local display name".to_string(),
            "/who -- open this device plus online and remembered peer details for the active channel".to_string(),
            "/send [path] -- share a file; a bare /send opens a file picker".to_string(),
            "/save [hash-prefix] -- download a shared file; each channel's files are kept in a folder of their own".to_string(),
            "/folder [path] -- show where saved files go and, with a path, move them all".to_string(),
            "/maxsize [size|off|default] -- show the biggest file you can share (500 MB to begin with) and, with a size like 2GB, change it (off lifts it, default restores it)".to_string(),
            format!("/autosave [on|off|limit <size|default>] -- /autosave on saves trusted peers' files in this channel as they arrive, up to its limit (the device-wide {autosave_limit} default unless this channel overrides it); /autosave limit <size> sets this channel's finite override and default restores its device-wide fallback"),
            "/outbox [on|off] -- /outbox on watches this channel's local send folder and shares new stable files once (off by default; existing, hidden, and temporary files are ignored)".to_string(),
            "/files -- list shared attachments and local synced files in this channel (type to filter, \u{2191}/\u{2193} pick, Enter save a shared file, Ctrl+O open, Ctrl+R reveal, Ctrl+E edit a local copy; never default-open a program or script, Esc close)".to_string(),
            "/paste -- share the clipboard".to_string(),
            "/sync [receive-only|publish-and-receive|off|pause|resume|include <glob>|exclude <glob>|remove <include|exclude> <glob>|clear-rules] -- /sync receive-only mirrors safe remote index entries into this channel's folder, while /sync publish-and-receive also publishes stable local files (off by default, per channel); /sync include and /sync exclude add relative glob rules, with excludes taking precedence; it never propagates deletes or renames".to_string(),
            "/conflicts [local|remote|both <path>] -- list unresolved sync collisions in this channel; /conflicts local|remote|both <path> resolves one by keeping the chosen copy (or both) and moving any displaced copy to recoverable sync trash".to_string(),
            "/trash [restore <path>|purge] -- list this channel's recoverable remote deletes; /trash restore <path> brings one back without overwriting a local file, and /trash purge permanently removes them all (trash is never automatically purged)".to_string(),
            "/alias <hex-prefix> <name> -- set a local name for a peer".to_string(),
            "/nick <name> -- set your broadcast display name".to_string(),
            "/search <term> (or /s) -- search channel messages".to_string(),
            "/reply [text] -- reply to the latest message".to_string(),
            "/thread -- view the latest reply thread".to_string(),
            "/hints -- toggle command hints; also toggles mouse reporting (hold Shift, or Option in iTerm2, to select text); hover a command hint to see its description and full usage".to_string(),
            "/bell -- toggle the message bell".to_string(),
            "/version -- show this build's version".to_string(),
            "/help -- show this help".to_string(),
        ];
        let keys = [
            "Tab/Shift+Tab -- switch channels",
            "Ctrl+P -- jump to a channel (type to filter, \u{2191}/\u{2193} pick, Enter confirm)",
            "Ctrl+K -- browse slash commands (type to filter, \u{2191}/\u{2193} pick, Enter run)",
            "/send file picker -- type to filter, \u{2191}/\u{2193} pick, Space mark, Enter open a folder or stage the marked files, Backspace up a level, Esc cancel",
            "Alt+Enter/Ctrl+J -- newline",
            "Ctrl+V -- share the clipboard (a file or image is staged first: Enter sends it, Backspace on an empty input unstages, Esc clears)",
            "Ctrl+R -- reply (\u{2191}/\u{2193} pick, Enter confirm)",
            "Ctrl+T -- thread (\u{2191}/\u{2193} pick, Enter confirm)",
            "Up/Down -- scroll",
            "Esc -- close /who peer details, the Ctrl+P jump-to list, or the Ctrl+K command picker; cancel an in-progress reply or thread pick, or an armed reply; otherwise clear an active search, clear staged attachments, leave an open thread, or quit",
            "Ctrl+C -- quit",
        ];
        self.push_system(format!(
            "commands:\n\n{}\n\nkeys:\n\n{}",
            commands.join("\n\n"),
            keys.join("\n\n"),
        ));
        None
    }

    /// Mouse hover is suppressed while another view owns input.
    pub fn can_hover_hints(&self) -> bool {
        self.show_hints
            && self.command_picker.is_none()
            && self.viewing_peers.is_none()
            && self.switching.is_none()
            && self.picking_file.is_none()
            && self.viewing_files.is_none()
            && self.pending_redeem.is_none()
            && self.picking_reply.is_none()
            && self.picking_thread.is_none()
    }

    /// Records the command-hints row measured under the mouse by the
    /// read-only UI layout (`ui::command_hint_at`); returns whether the
    /// frame changed.
    pub fn set_hint_hover(&mut self, row: Option<usize>) -> bool {
        let row = row.filter(|_| self.can_hover_hints());
        if self.hint_hover == row {
            return false;
        }
        self.hint_hover = row;
        true
    }

    /// The hovered command-hints row whose popup should show -- never
    /// while another view owns input, even before the next mouse event
    /// clears the hover.
    pub fn hovered_hint(&self) -> Option<usize> {
        self.hint_hover.filter(|_| self.can_hover_hints())
    }

    /// Records the peers-panel peer measured under the mouse by the
    /// read-only UI layout (`ui::peer_at`); returns whether the frame
    /// changed. Gated like `set_hint_hover`, since mouse reporting itself
    /// is only on while the hints panel is shown.
    pub fn set_peer_hover(&mut self, peer: Option<[u8; 32]>) -> bool {
        let peer = peer.filter(|_| self.can_hover_hints());
        if self.peer_hover == peer {
            return false;
        }
        self.peer_hover = peer;
        true
    }

    /// The hovered peer's popup details -- never while another view owns
    /// input, nor once that peer has left the active channel's list, even
    /// before the next mouse event clears the hover.
    pub fn hovered_peer(&self) -> Option<PeerDetails> {
        let id = self.peer_hover.filter(|_| self.can_hover_hints())?;
        let peer = self
            .peer_presence()
            .into_iter()
            .find(|peer| peer.id == id)?;
        Some(self.peer_details(&peer))
    }

    /// Everything the peers-panel popup shows about `peer`. Alias, nickname,
    /// and version are recorded per person (`canonical_id`), like
    /// `display_name`.
    pub fn peer_details(&self, peer: &PeerPresence) -> PeerDetails {
        let id = self.canonical_id(&peer.id);
        PeerDetails {
            id: peer.id,
            alias: self.petnames.get(&id).cloned(),
            nickname: self.nicknames.get(&id).cloned(),
            last_seen_unix_ms: peer.last_seen_unix_ms,
            version: self
                .peer_versions
                .get(&id)
                .map(|(version, _)| version.clone()),
        }
    }

    fn self_peer_details(&self) -> PeerDetails {
        PeerDetails {
            id: self.self_id,
            alias: None,
            nickname: self.local_nickname.clone(),
            last_seen_unix_ms: None,
            version: None,
        }
    }

    /// Handles `/hints`: toggles the sidebar's command hints panel (see
    /// `ui::render_sidebar`) and reports the new state as a system
    /// notice. Purely local UI state -- no `InputAction` needed, unlike
    /// commands that require the caller to do network I/O.
    fn run_hints(&mut self) -> Option<InputAction> {
        self.show_hints = !self.show_hints;
        self.set_hint_hover(None);
        self.set_peer_hover(None);
        let state = if self.show_hints { "shown" } else { "hidden" };
        self.push_system(format!("command hints {state}"));
        None
    }

    /// Handles `/bell`: toggles whether an incoming message rings the
    /// terminal bell (see main.rs's `ring_bell`) and reports the new state
    /// as a system notice, mirroring `run_hints`. Unlike `show_hints`,
    /// this preference is persisted (see settings.rs), so -- unlike
    /// `run_hints` -- an `InputAction` is returned for the caller to save
    /// it to disk.
    fn run_bell(&mut self) -> Option<InputAction> {
        self.bell_enabled = !self.bell_enabled;
        let state = if self.bell_enabled { "on" } else { "off" };
        self.push_system(format!("message bell {state}"));
        Some(InputAction::Bell(self.bell_enabled))
    }

    /// Handles `/alias <hex-prefix> <name>`: `arg` is everything after
    /// `/alias ` (already trimmed). Splits it into the hex-prefix and the
    /// (possibly multi-word) name to assign, resolves the prefix via
    /// `resolve_id`, and on success updates `petnames` immediately -- for
    /// instant UI feedback -- while returning an `InputAction::Alias` for
    /// the caller to persist (see contacts.rs). Usage and resolution
    /// errors are reported as a system notice instead.
    fn run_alias(&mut self, arg: &str) -> Option<InputAction> {
        let Some((prefix, name)) = arg.split_once(' ') else {
            self.push_system("usage: /alias <hex-prefix> <name>");
            return None;
        };
        let name = name.trim();
        if name.is_empty() {
            self.push_system("usage: /alias <hex-prefix> <name>");
            return None;
        }
        match self.resolve_id(prefix) {
            Ok(id) => {
                // Store under the canonical id (the user id, if a
                // certificate for `id` is already known) so the alias
                // applies to every one of that person's known devices, not
                // just the specific one whose hex prefix was typed --
                // while still confirming against the device id actually
                // typed, which is what the user recognizes.
                let canonical = self.canonical_id(&id);
                self.petnames.insert(canonical, name.to_string());
                self.push_system(format!("aliased {} as {name}", hex_id(&id)));
                Some(InputAction::Alias(canonical, name.to_string()))
            }
            Err(err) => {
                self.push_system(err);
                None
            }
        }
    }

    /// Handles `/pair` with no argument: builds a one-time pairing ticket
    /// for a brand-new device to redeem, and reports it back as a system
    /// notice, mirroring `/invite`. See `run_pair_redeem` for `/pair
    /// <ticket>`, which redeems one instead.
    fn run_pair(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            return Some(InputAction::Pair);
        }
        self.run_pair_redeem(arg)
    }

    /// Handles `/pair <ticket>`: validates `ticket` decodes (a pure parse,
    /// no I/O -- see this module's doc comment) and, if it does, arms
    /// `pending_redeem` and warns that confirming will replace this
    /// device's current identity, channels, and message history with the
    /// paired device's, naming how many channels are currently joined so
    /// the stakes are concrete rather than abstract. Deliberately rejects
    /// a malformed ticket immediately, before ever arming the
    /// confirmation or touching anything -- see main.rs's `restart_for_
    /// pairing` for what actually gets wiped once confirmed
    /// (`handle_redeem_confirmation_key`).
    fn run_pair_redeem(&mut self, ticket: &str) -> Option<InputAction> {
        match PairingTicket::decode_string(ticket) {
            Ok(_) => {
                self.pending_redeem = Some(ticket.to_string());
                self.push_system(format!(
                    "redeeming this ticket will ERASE this device's current identity, {} \
                     channel(s), and all local message history, replacing them with the \
                     paired device's -- this cannot be undone. Press Enter to confirm, Esc \
                     to cancel.",
                    self.channels.len()
                ));
            }
            Err(err) => {
                self.push_system(format!("invalid pairing ticket: {err}"));
            }
        }
        None
    }

    /// Handles `/forget-device <hex-prefix>`: resolves the prefix via
    /// `resolve_id` and, on success, immediately stops trusting that
    /// device locally -- for instant UI feedback -- while returning an
    /// `InputAction::ForgetDevice` for the caller to persist (see
    /// `crate::forgotten_devices`). Deliberately operates on the literal
    /// device id resolved, not its canonical user id (unlike `/alias`):
    /// forgetting targets one specific device (e.g. a lost phone), not
    /// every device belonging to that person. This is local and
    /// best-effort, not real revocation -- see
    /// `crate::forgotten_devices`'s doc comment.
    fn run_forget_device(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system("usage: /forget-device <hex-prefix>");
            return None;
        }
        match self.resolve_id(arg) {
            Ok(id) => {
                self.forgotten.insert(id);
                self.push_system(format!("forgot device {}", hex_id(&id)));
                Some(InputAction::ForgetDevice(id))
            }
            Err(err) => {
                self.push_system(err);
                None
            }
        }
    }

    /// Handles `/nick <name>`: `arg` is everything after `/nick ` (already
    /// trimmed). Unlike `/alias`, there's no id to resolve -- a `/nick` is
    /// about our own identity, not a peer's -- so a bare non-empty name is
    /// all that's needed. Reports the outcome as a system notice either
    /// way, and returns an `InputAction::Nick` for the caller to broadcast
    /// (see `net::Net::set_nickname`) on success.
    fn run_nick(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system("usage: /nick <name>");
            return None;
        }
        self.local_nickname = Some(arg.to_string());
        self.push_system(format!(
            "nickname set to {arg} (broadcast to joined channels)"
        ));
        Some(InputAction::Nick(arg.to_string()))
    }

    /// Handles `/leave [channel]`: `arg` is everything after `/leave `
    /// (already trimmed), naming the channel to leave -- as it appears on
    /// screen, see `resolve_channel_arg` -- or empty to leave the
    /// currently active one (mirroring `/invite`). Refuses -- with a
    /// system notice, returning `None` -- to leave a channel that isn't
    /// currently joined, or leyline's only remaining joined channel,
    /// since `AppState` always needs at least one (see `AppState::new`).
    /// On success, returns an `InputAction::Leave` for the caller to tear
    /// down; the tab itself is only removed once that's done, via
    /// `remove_channel`.
    fn run_leave(&mut self, arg: &str) -> Option<InputAction> {
        let target = if arg.is_empty() {
            self.active().name.clone()
        } else {
            match self.resolve_channel_arg(arg) {
                Ok(name) => name,
                Err(err) => {
                    self.push_system(err);
                    return None;
                }
            }
        };
        if self.channels.len() <= 1 {
            self.push_system("cannot leave your only channel");
            return None;
        }
        Some(InputAction::Leave(target))
    }

    /// Resolves a channel typed by the user (e.g. after `/leave`) to a
    /// joined channel's own `name`. Tries an exact match on `name` first,
    /// then on what's shown for it: its label (`Alice`) or its heading
    /// (`@Alice`, `#general`). The label differs from `name` for a DM or
    /// point-to-point chat, and the heading is how a channel is named in
    /// titles and notices. Errors on a reference shared by several
    /// channels -- a `/msg` DM and a `/dm` chat with the same person share
    /// a label, as do two peers given the same alias -- rather than
    /// guessing which was meant.
    fn resolve_channel_arg(&self, typed: &str) -> Result<String, String> {
        if self.channels.iter().any(|c| c.name == typed) {
            return Ok(typed.to_string());
        }
        let matches: Vec<&str> = self
            .channels
            .iter()
            .map(|c| c.name.as_str())
            .filter(|name| self.channel_label(name) == typed || self.channel_heading(name) == typed)
            .collect();
        match matches.as_slice() {
            [name] => Ok((*name).to_string()),
            [] => Err(format!("not currently in #{typed}")),
            _ => Err(format!(
                "'{typed}' matches more than one channel -- switch to the one you mean and run /leave with no argument"
            )),
        }
    }

    /// Handles `/rename [name]`: `arg` is everything after `/rename `
    /// (already trimmed). Gives the active *group* channel a local display
    /// name -- shown wherever the channel is named (`channel_label`), kept
    /// in `renames`, and never sent to peers -- and returns an
    /// `InputAction::Rename` for the caller to persist (see
    /// `crate::channel_renames`). With no name it restores the stored one
    /// instead, the way a bare `/search` clears a search, or shows usage
    /// when there's nothing to restore.
    ///
    /// Refuses a DM or point-to-point chat, whose label already follows the
    /// peer's `/alias` -- a second, independent override for the same
    /// conversation would only be confusing -- and a name that could be
    /// mistaken for another channel (`clashes_with_another_channel`).
    fn run_rename(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        if self.channel_kind(&channel) != ChannelKind::Group {
            self.push_system(
                "a conversation takes its name from the peer -- use /alias <hex-prefix> <name> to rename them",
            );
            return None;
        }
        if arg.is_empty() {
            if self.renames.remove(&channel).is_none() {
                self.push_system(
                    "usage: /rename <name> (a bare /rename restores the original name)",
                );
                return None;
            }
            self.push_system(format!("name restored to #{channel}"));
            return Some(InputAction::Rename {
                channel,
                name: None,
            });
        }
        if self.clashes_with_another_channel(&channel, arg) {
            self.push_system(format!(
                "another channel already goes by #{arg} -- pick a different name"
            ));
            return None;
        }
        self.renames.insert(channel.clone(), arg.to_string());
        self.push_system(format!(
            "renamed #{channel} to #{arg} (local only -- peers still see #{channel})"
        ));
        Some(InputAction::Rename {
            channel,
            name: Some(arg.to_string()),
        })
    }

    /// Whether `name` could be mistaken for a channel other than `channel`:
    /// it's another channel's stored name -- which `/leave` matches before
    /// anything displayed (`resolve_channel_arg`), so a stored name hidden
    /// behind a rename would otherwise shadow the one on screen -- or it's
    /// what another group channel already shows, which would leave two
    /// identical `#name` rows no one could tell apart. A conversation can
    /// share a name with a group channel: its `@` or bolt heading sets the
    /// two apart.
    fn clashes_with_another_channel(&self, channel: &str, name: &str) -> bool {
        self.channels
            .iter()
            .filter(|other| other.name != channel)
            .any(|other| {
                other.name == name
                    || (self.channel_kind(&other.name) == ChannelKind::Group
                        && self.channel_label(&other.name) == name)
            })
    }

    /// Handles `/who`: opens a local, dismissible table for the active
    /// channel's online and remembered peers. Presence comes from the same
    /// `peer_presence` view as the sidebar, while alias and nickname follow
    /// `peer_details`' canonical identity lookup. The table is a snapshot,
    /// so a focus change or network event cannot replace its data while it is
    /// open.
    fn run_who(&mut self) -> Option<InputAction> {
        let heading = self.channel_heading(&self.active().name);
        let presence = self.peer_presence();
        let first_offline = presence
            .iter()
            .position(|peer| peer.last_seen_unix_ms.is_some())
            .unwrap_or(presence.len());
        let mut peers: Vec<_> = presence[..first_offline]
            .iter()
            .map(|peer| self.peer_details(peer))
            .collect();
        peers.push(self.self_peer_details());
        peers.extend(
            presence[first_offline..]
                .iter()
                .map(|peer| self.peer_details(peer)),
        );
        self.viewing_peers = Some(PeersOverlay { heading, peers });
        None
    }

    /// Handles `/version`: reports our own build version and git commit,
    /// plus the newest peer version we've heard that's newer than ours, if
    /// any (see `newest_known_update`). Purely local, like `/who`/`/help`:
    /// no `InputAction` needed.
    fn run_version(&mut self) -> Option<InputAction> {
        let update = self
            .newest_known_update()
            .map(|(version, hash)| (version.to_string(), hash.to_string()));
        self.push_system(format!(
            "running v{} ({})",
            version::VERSION,
            version::GIT_HASH
        ));
        if let Some((peer_version, peer_hash)) = update {
            self.push_system(format!(
                "a peer is running a newer v{peer_version} ({peer_hash}) -- consider updating"
            ));
        }
        None
    }

    /// Handles `/search <term>` (`/s` is a shorthand alias, see
    /// `run_command`): `arg` is everything after the command name (already
    /// trimmed). With no argument, clears an active search filter -- or
    /// shows a usage hint if none is active -- mirroring `/hints`'s toggle
    /// shape. Otherwise scans the active channel's loaded transcript
    /// synchronously for an instant result (`Channel::search_loaded`),
    /// stores it as the channel's active search, and returns an
    /// `InputAction::Search` so the caller can extend it with a background
    /// scan of history older than what's loaded (see
    /// `search::run_on_disk_scan` and `AppState::apply_search_outcome`).
    /// The pane title (`ui::render_messages`) reminds you a search is
    /// active and how to clear it for as long as it stays active, since
    /// this system notice will otherwise scroll out of view.
    fn run_search(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            if !self.clear_active_search() {
                self.push_system("usage: /search <term> (or /s <term>)");
            }
            return None;
        }
        let term_lower = arg.to_lowercase();
        let messages = self.active().search_loaded(&term_lower);
        let count = messages.len();
        self.active_mut().thread = None;
        self.active_mut().search = Some(SearchResults {
            term: arg.to_string(),
            messages,
            pending: true,
        });
        self.push_system(format!(
            "search: {count} match(es) so far for '{arg}' (scanning full history...) -- /s to clear"
        ));
        Some(InputAction::Search {
            channel: self.active().name.clone(),
            term: arg.to_string(),
        })
    }

    /// Finds the newest message currently visible in the active channel --
    /// shared by reply targeting (`newest_reply_candidate`) and thread
    /// targeting (`newest_thread_candidate`), which differ only in which
    /// Handles `/send [path]`: `arg` is everything after `/send ` (already
    /// trimmed). With no path, opens the file picker (`open_picker`) to
    /// choose files in. With one, it names a local file to share with the
    /// active channel right away: strips one layer of surrounding quotes
    /// (see `strip_quotes`) and otherwise only checks that a path was given
    /// -- app.rs never touches the filesystem (see this module's doc
    /// comment), so the path itself (existence, `~`-expansion, etc.) is
    /// validated and resolved by the caller (`net::Net::send_file`) once
    /// this returns. Pushes an immediate breadcrumb since importing and
    /// broadcasting a file happens in the background and may take a moment.
    fn run_send(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            return Some(self.open_picker());
        }
        let path = strip_quotes(arg);
        let name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        self.push_system(format!("sending {name}..."));
        Some(InputAction::SendFile {
            channel: self.active().name.clone(),
            path: path.to_string(),
        })
    }

    /// Handles `/save [hash-prefix]`: `arg` is everything after `/save `
    /// (already trimmed). With a prefix, it names (a prefix of) a file
    /// previously shared in any joined channel -- see `resolve_attachment`,
    /// which mirrors `/alias`'s hex-prefix resolution for a consistent UX.
    /// With none, it takes the newest file another peer shared in the
    /// active channel (`newest_shared_file`), mirroring a bare `/reply`'s
    /// "most recent message" default -- name an older file by its prefix.
    /// Either way the file goes through the same `InputAction::SaveFile`,
    /// asking its sender and every other peer known where it was shared
    /// (`providers_for`), not only the sender, and landing in the folder of
    /// the channel it was shared in (`channel_of_file`).
    fn run_save(&mut self, arg: &str) -> Option<InputAction> {
        let file = if arg.is_empty() {
            self.newest_shared_file()
        } else {
            self.resolve_attachment(arg)
        };
        match file {
            Ok((hash, filename, sender)) => {
                let channel = self.channel_of_file(hash).to_string();
                Some(self.save_request(channel, hash, filename, sender))
            }
            Err(err) => {
                self.push_system(err);
                None
            }
        }
    }

    /// Builds the `InputAction::SaveFile` that fetches the file `hash` into
    /// `channel`'s folder, and leaves the `saving ...` notice -- the one
    /// place that is done, so `/save` and `Enter` in the files overlay can't
    /// differ. `filename` and `sender` are as `resolve_attachment` gives
    /// them. The file is asked of its sender first and then of every other
    /// peer known where it was shared (`providers_for`), and goes into the
    /// folder (`files::channel_dir`) of `channel`, a stored name. Someone
    /// asked for this file and can see what it is, so it is a
    /// `SaveMode::Manual` save: no limit on its size.
    fn save_request(
        &mut self,
        channel: String,
        hash: Hash,
        filename: String,
        sender: [u8; 32],
    ) -> InputAction {
        let providers = self.providers_for(hash, sender);
        let dir = self.folder_dir(&channel);
        self.push_system(format!("saving {filename}..."));
        InputAction::SaveFile(SaveRequest {
            channel,
            hash,
            filename,
            providers,
            dir,
            mode: SaveMode::Manual,
        })
    }

    /// Decides what to do about a file that has just arrived live in
    /// `channel` (a stored name). main.rs calls this for each message newly
    /// accepted off the network (`handle_net_event`), and for nothing else:
    /// history loaded at startup and history backfilled from a peer never
    /// come here, so a file shared before this session, while auto-save was
    /// off, or before this device joined is never fetched on its own.
    /// `already_saved` says whether the channel's folder already holds a copy
    /// of a file, which app.rs can't look at.
    ///
    /// Only a file from another device, in a channel with auto-save on
    /// (`/autosave`), is any concern of this; for anything else it says
    /// nothing. For one that is, it says why and stops when the sender isn't
    /// trusted (`is_trusted_peer`), the file is over the limit
    /// (`AUTOSAVE_MAX_FILE_SIZE`) or the channel already holds a copy, and
    /// otherwise returns the `SaveRequest`. Every notice goes to `channel`'s
    /// transcript, wherever the user is looking, and points to the `/save`
    /// that fetches the file by hand.
    ///
    /// `message.sender` is only what the sender wrote: nothing signs a chat
    /// message, so anyone in the channel can claim to be someone else. It
    /// decides whether to try at all and is not what keeps strangers out --
    /// the providers are. The file is only ever asked of peers that are
    /// themselves trusted, who are known by the connection to them rather
    /// than by what a message says, so a forged share of a file that only a
    /// stranger has fetches nothing. The download is refused, too, if the
    /// file turns out bigger than its sender said (`SaveMode::Automatic`).
    pub fn auto_save(
        &mut self,
        channel: &str,
        message: &ChatMessage,
        already_saved: impl Fn(Hash) -> bool,
    ) -> Option<SaveRequest> {
        let file = message.attachment.as_ref()?;
        if message.sender == self.self_id || !self.autosave.contains(channel) {
            return None;
        }
        let hash = file.hash;
        let by_hand = format!("{} to download it yourself", save_hint(hash));
        if !self.is_trusted_peer(&message.sender) {
            self.push_system_to(
                channel,
                format!(
                    "not auto-saving {} from {}: auto-save only takes files from your own devices and the peers you have aliased -- {by_hand}",
                    file.filename,
                    self.display_name(&message.sender)
                ),
            );
            return None;
        }
        let max_bytes = self.autosave_limit_for(channel).max_bytes();
        if file.size > max_bytes {
            self.push_system_to(
                channel,
                format!(
                    "not auto-saving {} ({}): over the {} auto-save limit -- {by_hand}",
                    file.filename,
                    human_size(file.size),
                    human_size(max_bytes)
                ),
            );
            return None;
        }
        if already_saved(hash) {
            self.push_system_to(
                channel,
                format!(
                    "not auto-saving {}: this channel already has a saved copy of it",
                    file.filename
                ),
            );
            return None;
        }
        if !self.auto_saving.insert((channel.to_string(), hash)) {
            // The same file is being fetched right now: its notice will come.
            return None;
        }
        let providers = self
            .providers_for(hash, message.sender)
            .into_iter()
            .filter(|id| self.is_trusted_peer(id))
            .collect();
        Some(SaveRequest {
            channel: channel.to_string(),
            hash,
            filename: file.filename.clone(),
            providers,
            dir: self.folder_dir(channel),
            mode: SaveMode::Automatic { max_bytes },
        })
    }

    /// Whether files from `id` may be saved without anyone asking: one of our
    /// own devices -- certified under our own user key, so it resolves
    /// (`canonical_id`) to `self_user_id`; a device forgotten with
    /// `/forget-device` doesn't -- or a peer we have given an alias
    /// (`petnames`, keyed by the same resolved identity, so an alias set on
    /// any one of a person's devices covers them all). The one answer both
    /// halves of auto-save go by (`auto_save`).
    fn is_trusted_peer(&self, id: &[u8; 32]) -> bool {
        let identity = self.canonical_id(id);
        identity == self.self_user_id || self.petnames.contains_key(&identity)
    }

    /// Handles `/folder [path]`: `arg` is everything after `/folder `
    /// (already trimmed). With no path, says where received files go: the
    /// active channel's folder -- named from its stored name, so `/rename`
    /// and `/alias` don't move it -- and the files root it is under. With
    /// one, moves the files root there: strips one layer of quotes, as
    /// `/send` does, and asks the caller to do it
    /// (`InputAction::SetFilesRoot`), since whether the path will do depends
    /// on the filesystem, which app.rs never touches.
    fn run_folder(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            let channel = self.active().name.clone();
            self.push_system(format!(
                "files from {} are saved in {} -- every channel's folder is under {}, which /folder <path> moves",
                self.channel_heading(&channel),
                self.folder_dir(&channel).display(),
                self.files_root.display()
            ));
            return None;
        }
        Some(InputAction::SetFilesRoot(strip_quotes(arg).to_string()))
    }

    /// Handles `/maxsize [<size>|off|default]`: `arg` is everything after
    /// `/maxsize ` (already trimmed). With nothing, says what the limit on
    /// the size of a shared file is and how to change it, as a bare `/folder`
    /// says where files go. With something, reads it
    /// (`SizeLimit::parse`, the one place it is validated: what it refuses
    /// leaves the limit as it was), applies it at once, and asks the caller
    /// to remember it (`InputAction::SetSizeLimit`), as `/bell` does. There
    /// is nothing else to tell: every route a file takes to being shared
    /// checks `size_limit` in main.rs.
    fn run_maxsize(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system(self.size_limit_notice());
            return None;
        }
        match SizeLimit::parse(arg) {
            Ok(limit) => {
                self.size_limit = limit;
                self.push_system(self.size_limit_notice());
                Some(InputAction::SetSizeLimit(limit))
            }
            Err(message) => {
                self.push_system(message);
                None
            }
        }
    }

    /// Handles `/autosave [on|off|limit <size|default>]`: `arg` is everything after `/autosave `
    /// (already trimmed). With nothing, says whether auto-save is on for the
    /// active channel -- it is off until turned on, for every channel -- and
    /// what it would do. With `on` or `off`, switches it for the active
    /// channel (a group channel, `/msg` DM or `/dm` chat alike) and asks the
    /// caller to remember it (`InputAction::SetAutoSave`), but only if that
    /// changed anything: asking for what already holds writes nothing.
    /// Anything else is usage, and changes nothing.
    fn run_autosave(&mut self, arg: &str) -> Option<InputAction> {
        if let Some(size) = arg.strip_prefix("limit ") {
            return match AutoSaveLimit::parse(size) {
                Ok(limit) => {
                    let channel = self.active().name.clone();
                    let override_limit = match limit {
                        AutoSaveLimit::Default => None,
                        limit => Some(limit),
                    };
                    match override_limit {
                        Some(limit) => {
                            self.autosave_limits.insert(channel.clone(), limit);
                        }
                        None => {
                            self.autosave_limits.remove(&channel);
                        }
                    }
                    self.push_system(self.autosave_limit_notice());
                    Some(InputAction::SetChannelAutoSaveLimit {
                        channel,
                        limit: override_limit,
                    })
                }
                Err(message) => {
                    self.push_system(message);
                    None
                }
            };
        }
        let channel = self.active().name.clone();
        let on = match arg {
            "" => {
                self.push_system(self.autosave_notice(&channel));
                return None;
            }
            "on" => true,
            "off" => false,
            _ => {
                self.push_system("usage: /autosave [on|off|limit <size|default>]");
                return None;
            }
        };
        let changed = if on {
            self.autosave.insert(channel.clone())
        } else {
            self.autosave.remove(&channel)
        };
        self.push_system(self.autosave_notice(&channel));
        changed.then_some(InputAction::SetAutoSave { channel, on })
    }

    /// Handles `/outbox [on|off]`. The setting is only applied once the
    /// runtime has created and snapshot the folder successfully.
    fn run_outbox(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        let on = match arg {
            "" => {
                self.push_system(self.outbox_notice(&channel));
                return None;
            }
            "on" => true,
            "off" => false,
            _ => {
                self.push_system("usage: /outbox [on|off]");
                return None;
            }
        };
        if self.outbox.contains(&channel) == on {
            self.push_system(self.outbox_notice(&channel));
            return None;
        }
        Some(InputAction::SetOutbox { channel, on })
    }

    /// Handles per-channel `/sync` policy and path-rule configuration.
    fn run_sync(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        let mut config = self.sync_config(&channel);
        let changed = match arg {
            "" => {
                self.push_system(self.sync_notice(&channel));
                return None;
            }
            "on" | "publish-and-receive" => {
                config.policy = SyncPolicy::PublishAndReceive;
                config.paused = false;
                true
            }
            "receive-only" => {
                config.policy = SyncPolicy::ReceiveOnly;
                config.paused = false;
                true
            }
            "off" => {
                config.policy = SyncPolicy::Off;
                config.paused = false;
                true
            }
            "pause" if config.policy != SyncPolicy::Off => {
                config.paused = true;
                true
            }
            "resume" if config.policy != SyncPolicy::Off => {
                config.paused = false;
                true
            }
            "pause" | "resume" => {
                self.push_system("sync is off -- select receive-only or publish-and-receive first");
                return None;
            }
            "clear-rules" => config.clear_rules(),
            _ => {
                let (operation, pattern) = if let Some(pattern) = arg.strip_prefix("include ") {
                    ("include", pattern)
                } else if let Some(pattern) = arg.strip_prefix("exclude ") {
                    ("exclude", pattern)
                } else if let Some(rule) = arg.strip_prefix("remove include ") {
                    ("remove include", rule)
                } else if let Some(rule) = arg.strip_prefix("remove exclude ") {
                    ("remove exclude", rule)
                } else {
                    self.push_system(
                        "usage: /sync [receive-only|publish-and-receive|off|pause|resume|include <glob>|exclude <glob>|remove <include|exclude> <glob>|clear-rules]",
                    );
                    return None;
                };
                let rule = match operation {
                    "include" | "remove include" => SyncRule::include(pattern),
                    "exclude" | "remove exclude" => SyncRule::exclude(pattern),
                    _ => unreachable!(),
                };
                match rule {
                    Ok(rule) if operation.starts_with("remove") => config.remove_rule(&rule),
                    Ok(rule) => config.add_rule(rule),
                    Err(error) => {
                        self.push_system(error);
                        return None;
                    }
                }
            }
        };
        if !changed || self.sync_config(&channel) == config {
            self.push_system(self.sync_notice(&channel));
            return None;
        }
        self.sync.insert(channel.clone(), config.clone());
        self.push_system(self.sync_notice(&channel));
        Some(InputAction::SetSync { channel, config })
    }

    /// Handles `/trash [restore <path>|purge]` for the active channel.
    fn run_trash(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        match arg {
            "" => Some(InputAction::ListSyncTrash(channel)),
            "purge" => Some(InputAction::PurgeSyncTrash(channel)),
            _ => match arg.strip_prefix("restore ") {
                Some(path) if !path.trim().is_empty() => Some(InputAction::RestoreSyncTrash {
                    channel,
                    path: path.trim().to_string(),
                }),
                _ => {
                    self.push_system("usage: /trash [restore <path>|purge]");
                    None
                }
            },
        }
    }

    fn run_conflicts(&mut self, arg: &str) -> Option<InputAction> {
        let channel = self.active().name.clone();
        let conflicts = self.sync_conflicts(&channel);
        if arg.is_empty() {
            if conflicts.is_empty() {
                self.push_system(format!(
                    "no unresolved sync conflicts in {}",
                    self.channel_heading(&channel)
                ));
            } else {
                self.push_system(format!(
                    "unresolved sync conflicts in {} ({}):",
                    self.channel_heading(&channel),
                    conflicts.len()
                ));
                for conflict in &conflicts {
                    self.push_system(self.conflict_notice(conflict));
                }
                self.push_system(
                    "resolve with /conflicts local|remote|both <local path> -- displaced copies go to sync trash",
                );
            }
            return None;
        }
        let Some((choice, path)) = arg.split_once(' ') else {
            self.push_system("usage: /conflicts [local|remote|both <local path>]");
            return None;
        };
        let resolution = match choice {
            "local" => ConflictResolution::Local,
            "remote" => ConflictResolution::Remote,
            "both" => ConflictResolution::Both,
            _ => {
                self.push_system("usage: /conflicts [local|remote|both <local path>]");
                return None;
            }
        };
        let path = path.trim();
        let Some(conflict) = conflicts
            .into_iter()
            .find(|conflict| conflict.local_path == path)
        else {
            self.push_system(format!("no unresolved sync conflict for {path:?}"));
            return None;
        };
        Some(InputAction::ResolveSyncConflict {
            channel,
            conflict,
            resolution,
        })
    }
    /// What a bare `/sync` reports for `channel`: its persisted policy,
    /// folder, rules and runtime status, once sync has actually been
    /// configured here -- or, the first time anyone asks, a short setup
    /// notice instead (`sync_onboarding_notice`), so exploring the command
    /// on a brand-new channel doesn't read like sync is already on and
    /// idle.
    fn sync_notice(&self, channel: &str) -> String {
        if !self.sync.contains_key(channel) {
            return self.sync_onboarding_notice(channel);
        }
        let heading = self.channel_heading(channel);
        let folder = self.folder_dir(channel);
        let status = self.sync_status(channel);
        let pending = status.map_or(0, SyncStatus::pending_count);
        let errors = status.map_or(0, SyncStatus::error_count);
        let conflicts = status.map_or(0, SyncStatus::conflict_count);
        let last_success = status
            .and_then(|status| status.last_success_unix_ms)
            .map(|timestamp| format!("last success: {timestamp}"))
            .unwrap_or_else(|| "last success: never".to_string());
        let mut report = format!(
            "sync is {} for {heading} -- folder: {}; rules: {}; {pending} pending; {conflicts} conflict(s); {errors} error(s); {last_success}",
            self.sync_config(channel).label(),
            folder.display(),
            self.sync_rule_notice(channel),
        );
        if let Some(status) = status {
            for conflict in &status.conflicts {
                report.push_str(&format!("; conflict: {}", self.conflict_notice(conflict)));
            }
            for (path, error) in &status.errors {
                report.push_str(&format!("; error: {path}: {error}"));
            }
        }
        if conflicts > 0 {
            report.push_str(
                " -- resolve with /conflicts local|remote|both <local path> -- displaced copies go to sync trash",
            );
        }
        if pending > 0 {
            report.push_str(
                " -- pending files need their peer online; check who's reachable with /who",
            );
        }
        report
    }

    /// First-use `/sync` guidance for a channel whose sync has never been
    /// configured: where its folder will be, and what the other side needs
    /// to do too -- the detail a bare status report has no room for. A
    /// group channel needs an invite first; a DM or point-to-point chat
    /// already has its one peer, so it only needs turning on both sides.
    fn sync_onboarding_notice(&self, channel: &str) -> String {
        let heading = self.channel_heading(channel);
        let folder = self.folder_dir(channel);
        let folder = folder.display();
        match self.channel_kind(channel) {
            ChannelKind::Group => format!(
                "sync is off for {heading} -- its folder will be {folder} once enabled; invite someone into this group channel with /invite, have them /join, then run /sync publish-and-receive here and on their side (or /sync receive-only to only receive)"
            ),
            ChannelKind::GossipDm { .. } | ChannelKind::Direct { .. } => format!(
                "sync is off for {heading} -- its folder will be {folder} once enabled; run /sync publish-and-receive here and ask them to do the same on their side (or /sync receive-only to only receive)"
            ),
        }
    }

    fn conflict_notice(&self, conflict: &SyncConflict) -> String {
        let local_time = conflict
            .local_modified_unix_ms
            .map_or_else(|| "unknown".to_string(), |time| time.to_string());
        let remote_time = conflict
            .remote_modified_unix_ms
            .map_or_else(|| "unknown".to_string(), |time| time.to_string());
        let author = conflict
            .author
            .map(|id| self.display_name(&id))
            .unwrap_or_else(|| "unknown".to_string());
        let remote_copy = conflict
            .remote_copy_path
            .as_deref()
            .unwrap_or("remote delete");
        format!(
            "local {} (modified {local_time}) vs remote {} ({remote_copy}, modified {remote_time}, author {author})",
            conflict.local_path, conflict.remote_path
        )
    }

    fn sync_rule_notice(&self, channel: &str) -> String {
        let rules = &self.sync_config(channel).rules;
        if rules.is_empty() {
            "all paths".to_string()
        } else {
            rules
                .iter()
                .map(|rule| format!("{} {}", rule.label(), rule.pattern()))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    /// What `/outbox` says about the active channel's local one-way folder.
    fn outbox_notice(&self, channel: &str) -> String {
        let heading = self.channel_heading(channel);
        let folder = self.folder_dir(channel).join("send");
        if self.outbox.contains(channel) {
            format!(
                "outbox is on for {heading} -- new stable files in {} are shared once; /outbox off turns it off",
                folder.display()
            )
        } else {
            format!(
                "outbox is off for {heading} -- /outbox on watches {} for new files to share",
                folder.display()
            )
        }
    }

    /// What `/autosave` says about `channel`: whether auto-save is on for it,
    /// whose files it takes, up to how big, and where they go -- the same
    /// words for asking and for having just switched it.
    fn autosave_notice(&self, channel: &str) -> String {
        let heading = self.channel_heading(channel);
        let limit = human_size(self.autosave_limit_for(channel).max_bytes());
        let folder = channel_dir(&self.files_root, channel);
        let folder = folder.display();
        if self.autosave.contains(channel) {
            format!(
                "auto-save is on for {heading} -- files from your own devices and the peers you have aliased, up to {limit}, are saved into {folder} as they arrive; /autosave off turns it off"
            )
        } else {
            format!(
                "auto-save is off for {heading} -- /autosave on saves files from your own devices and the peers you have aliased, up to {limit}, into {folder} as they arrive"
            )
        }
    }

    fn autosave_limit_notice(&self) -> String {
        let channel = &self.active().name;
        match self.autosave_limits.get(channel) {
            Some(limit) => format!(
                "automatic downloads in this channel are limited to {} -- /autosave limit default restores the device-wide fallback",
                human_size(limit.max_bytes())
            ),
            None => format!(
                "automatic downloads in this channel use the device-wide {} limit -- /autosave limit <size> sets an override",
                human_size(self.autosave_limit.max_bytes())
            ),
        }
    }

    /// What `/maxsize` says about the limit as it stands, and how to change
    /// it -- the same words for asking and for having just changed it.
    fn size_limit_notice(&self) -> String {
        let default = human_size(DEFAULT_MAX_FILE_SIZE);
        match self.size_limit {
            SizeLimit::Default => format!(
                "files over {default} are refused (the default) -- /maxsize <size> raises or lowers it, /maxsize off lifts it"
            ),
            SizeLimit::Bytes(bytes) => format!(
                "files over {} are refused (set with /maxsize) -- /maxsize <size> changes it, /maxsize off lifts it, /maxsize default restores the {default} default",
                human_size(bytes)
            ),
            SizeLimit::Off => format!(
                "there is no limit on the size of a file you share (set with /maxsize) -- /maxsize <size> sets one, /maxsize default restores the {default} default"
            ),
        }
    }

    /// Handles `/paste` (and the `Ctrl+V`/`Cmd+V` keybinding in
    /// `handle_key`): shares whatever's currently on the OS clipboard with
    /// the active channel. Takes no argument -- like `/invite`, it always
    /// acts on the active channel. app.rs never touches the clipboard
    /// itself (see this module's doc comment); the caller
    /// (`net::Net::paste`) reads it and reports back what it found (a
    /// file, an image, or plain text) as a `NetEvent`. Pushes an immediate
    /// breadcrumb since reading the clipboard and importing its contents
    /// happens in the background and may take a moment, mirroring
    /// `run_send`.
    fn run_paste(&mut self) -> Option<InputAction> {
        self.push_system("pasting from clipboard...");
        Some(InputAction::Paste(self.active().name.clone()))
    }

    /// Handles `/msg <alias-or-hex-prefix> [text]`: `arg` is everything
    /// after `/msg ` (already trimmed), split on the first space into a
    /// target (resolved via `resolve_msg_target`) and an optional message
    /// to send immediately, mirroring `/reply [text]`'s shape. If a DM
    /// channel is already joined for that peer (`dm_channels`, see
    /// `crate::dm_registry`), just switches to it -- else picks a fresh
    /// local name (`unique_dm_channel_name`) and returns an
    /// `InputAction::StartDm` for the caller to create it (see
    /// `net::Net::join` and `NetEvent::Joined`/`DmJoined`). A recorded DM
    /// channel that isn't currently joined (e.g. left outside of `/leave`)
    /// is treated the same as none at all -- `activate_existing_channel`
    /// returning `false` falls through to creating a new one.
    fn run_msg(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system("usage: /msg <alias-or-hex-prefix> [text]");
            return None;
        }
        let (target, text) = match arg.split_once(' ') {
            Some((target, rest)) => (target, Some(rest.trim()).filter(|s| !s.is_empty())),
            None => (arg, None),
        };
        let id = match self.resolve_msg_target(target) {
            Ok(id) => id,
            Err(err) => {
                self.push_system(err);
                return None;
            }
        };

        if let Some(name) = self.dm_channels.get(&id).cloned()
            && self.activate_existing_channel(&name)
        {
            self.push_system(format!(
                "switched to your DM with {}",
                self.display_name(&id)
            ));
            return text.map(|text| self.send_text(text, None));
        }

        let channel = self.unique_dm_channel_name(&id);
        self.push_system(format!(
            "starting a DM with {} in #{channel} -- share the invite once it's ready",
            self.display_name(&id)
        ));
        Some(InputAction::StartDm {
            peer: id,
            channel,
            text: text.map(str::to_string),
        })
    }

    /// Handles `/dm <alias|hex-prefix|endpoint-id> [text]`: a true
    /// point-to-point conversation, with no gossip channel, room secret,
    /// or invite ticket involved (see `direct.rs`) -- `/msg` is the
    /// gossip-based equivalent. `arg` is everything after `/dm ` (already
    /// trimmed), split the same way `run_msg` splits its argument.
    /// Resolves the target via `resolve_dm_target`, which -- unlike
    /// `resolve_msg_target` -- also accepts a full endpoint id for a peer
    /// never otherwise observed, so two people who already know each
    /// other's id can start talking without having shared a channel
    /// first. If a point-to-point tab is already open for that peer
    /// (`direct_channels`), just switches to it -- else returns
    /// `InputAction::OpenDirect` for the caller (main.rs) to ensure the
    /// connection and create the tab, since unlike `run_msg`'s gossip
    /// subscribe, there's nothing here worth doing synchronously ahead of
    /// that (`net::Net::open_direct` is itself synchronous).
    fn run_dm(&mut self, arg: &str) -> Option<InputAction> {
        if arg.is_empty() {
            self.push_system("usage: /dm <alias|hex-prefix|endpoint-id> [text]");
            return None;
        }
        let (target, text) = match arg.split_once(' ') {
            Some((target, rest)) => (target, Some(rest.trim()).filter(|s| !s.is_empty())),
            None => (arg, None),
        };
        let id = match self.resolve_dm_target(target) {
            Ok(id) => id,
            Err(err) => {
                self.push_system(err);
                return None;
            }
        };
        if id == self.self_id {
            self.push_system("you can't start a point-to-point chat with yourself");
            return None;
        }

        if let Some(name) = self.direct_channels.get(&id).cloned()
            && self.activate_existing_channel(&name)
        {
            self.push_system(format!(
                "switched to your point-to-point chat with {}",
                self.display_name(&id)
            ));
            return text.map(|text| self.send_text(text, None));
        }

        self.push_system(format!(
            "starting a point-to-point chat with {}...",
            self.display_name(&id)
        ));
        Some(InputAction::OpenDirect {
            peer: id,
            text: text.map(str::to_string),
        })
    }

    /// Resolves a `/msg` target: an exact (case-insensitive) local pet
    /// name first -- checked against every saved pet name, not just
    /// `known_ids()`, since you should be able to `/msg` someone you've
    /// aliased even if they aren't observed anywhere right now -- else a
    /// hex-prefix via `resolve_id` (which _is_ scoped to `known_ids()`,
    /// since there's nothing else to match a raw prefix against). Errors
    /// mirror `resolve_id`'s for a consistent UX.
    fn resolve_msg_target(&self, target: &str) -> Result<[u8; 32], String> {
        let matches: Vec<[u8; 32]> = self
            .petnames
            .iter()
            .filter(|(_, name)| name.eq_ignore_ascii_case(target))
            .map(|(id, _)| *id)
            .collect();
        match matches.as_slice() {
            [id] => Ok(*id),
            [] => self.resolve_id(target),
            _ => Err(format!(
                "'{target}' matches multiple aliased peers, use a hex prefix instead"
            )),
        }
    }

    /// Resolves a `/dm` target: `resolve_msg_target`'s pet-name/hex-prefix
    /// resolution first, falling back to parsing `target` itself as a
    /// full, not-necessarily-already-known endpoint id (exactly 64 hex
    /// characters) if that fails. This fallback is what lets `/dm` reach
    /// a peer who's never appeared in any joined channel or been
    /// `/alias`ed -- the whole point of a point-to-point connection being
    /// usable the moment two people already know each other's endpoint
    /// id, with nothing else shared yet.
    fn resolve_dm_target(&self, target: &str) -> Result<[u8; 32], String> {
        match self.resolve_msg_target(target) {
            Ok(id) => Ok(id),
            Err(err) => decode_hex_id(target).ok_or(err),
        }
    }

    /// Picks a not-currently-joined local channel name for a new DM with
    /// `id`: `dm-<label>` (their pet name if set, else their hex prefix),
    /// suffixed with `-2`, `-3`, ... on the rare chance that's already
    /// taken by an unrelated channel.
    fn unique_dm_channel_name(&self, id: &[u8; 32]) -> String {
        let label = self
            .petnames
            .get(id)
            .cloned()
            .unwrap_or_else(|| hex_prefix(id));
        let base = format!("dm-{label}");
        if !self.channels.iter().any(|c| c.name == base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|name| !self.channels.iter().any(|c| &c.name == name))
            .expect("an unbounded suffix sequence always finds a free name")
    }

    /// Resolves a hex-prefix (case-insensitive, as typed after `/alias`)
    /// to exactly one id in `known_ids`. Errors with a user-facing message
    /// if the prefix isn't valid hex, matches no known id, or matches more
    /// than one -- this is what makes `/alias` "do nothing for a peer you
    /// haven't met yet" rather than guessing.
    fn resolve_id(&self, prefix: &str) -> Result<[u8; 32], String> {
        let prefix = prefix.to_lowercase();
        let valid_hex = !prefix.is_empty()
            && prefix.len() <= 64
            && prefix.chars().all(|c| c.is_ascii_hexdigit());
        if !valid_hex {
            return Err(format!("invalid hex prefix: {prefix}"));
        }
        let matches: Vec<[u8; 32]> = self
            .known_ids()
            .into_iter()
            .filter(|id| hex_id(id).starts_with(&prefix))
            .collect();
        match matches.as_slice() {
            [] => Err(format!("no known peer matches '{prefix}'")),
            [id] => Ok(*id),
            _ => Err(format!(
                "'{prefix}' matches multiple peers, use a longer prefix"
            )),
        }
    }

    /// Every endpoint id `AppState` has actually observed: everyone
    /// currently online in any joined channel, plus the sender of any
    /// chat message in any channel's transcript (including messages
    /// loaded from history) -- i.e. everyone `/alias` could plausibly
    /// name, whether or not they're online right now. Excludes our own
    /// id, and is deduplicated.
    fn known_ids(&self) -> Vec<[u8; 32]> {
        let mut ids: Vec<[u8; 32]> = Vec::new();
        for channel in &self.channels {
            ids.extend(channel.peers.iter().copied());
            for line in &channel.messages {
                if let TranscriptLine::Chat(message) = line {
                    ids.push(message.sender);
                }
            }
        }
        ids.retain(|id| *id != self.self_id);
        ids.sort();
        ids.dedup();
        ids
    }

    /// Resolves a hex-prefix (case-insensitive, as typed after `/save`) to
    /// exactly one attachment in `known_attachments`, returning its hash,
    /// filename, and original sender (the first peer `/save` asks for it --
    /// see `providers_for`). Mirrors `resolve_id`'s errors for a
    /// consistent UX: invalid hex, no match, or an ambiguous prefix.
    fn resolve_attachment(&self, prefix: &str) -> Result<(Hash, String, [u8; 32]), String> {
        let prefix = prefix.to_lowercase();
        let valid_hex = !prefix.is_empty()
            && prefix.len() <= 64
            && prefix.chars().all(|c| c.is_ascii_hexdigit());
        if !valid_hex {
            return Err(format!("invalid hash prefix: {prefix}"));
        }
        let matches: Vec<(Hash, String, [u8; 32])> = self
            .known_attachments()
            .into_iter()
            .filter(|(hash, _, _)| hash.to_hex().starts_with(&prefix))
            .collect();
        match matches.as_slice() {
            [] => Err(format!("no known file matches '{prefix}'")),
            [one] => Ok(one.clone()),
            _ => Err(format!(
                "'{prefix}' matches multiple files, use a longer prefix"
            )),
        }
    }

    /// Every file attachment `AppState` has seen in any joined channel's
    /// transcript (including history loaded at startup), deduplicated by
    /// hash -- i.e. everything `/save` could plausibly resolve. Mirrors
    /// `known_ids`'s shape.
    fn known_attachments(&self) -> Vec<(Hash, String, [u8; 32])> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();
        for channel in &self.channels {
            if let Some(files) = self.shared_files.get(&channel.name) {
                for shared in files {
                    if seen.insert(shared.file.hash) {
                        result.push((
                            shared.file.hash,
                            shared.file.filename.clone(),
                            shared.sender,
                        ));
                    }
                }
            }
        }
        result
    }

    /// The newest file another peer shared in the active channel -- what a
    /// bare `/save` fetches, as a bare `/reply` answers the newest message
    /// -- as `resolve_attachment` returns one: its hash, filename, and its
    /// sender. "Another peer" is anyone but this device
    /// (`self_id`), the test `ui::attachment_line` uses to give a file its
    /// `/save <hash>` hint, so this is always the newest file that shows
    /// one -- including a file from one of our own other devices, which
    /// isn't on this one. Looks through the channel's loaded transcript,
    /// like `known_attachments`, not `Channel::visible_chat_ids`: an open
    /// thread or search narrows what `/reply` can target, but the file just
    /// shared belongs to the channel, not to the view. Errors with a
    /// user-facing message if there is none.
    fn newest_shared_file(&self) -> Result<(Hash, String, [u8; 32]), String> {
        self.shared_files
            .get(&self.active().name)
            .into_iter()
            .flatten()
            .find(|shared| shared.sender != self.self_id)
            .map(|shared| {
                (
                    shared.file.hash,
                    shared.file.filename.clone(),
                    shared.sender,
                )
            })
            .ok_or_else(|| "no file from another peer to save in this channel".to_string())
    }

    /// The channel a saved file belongs to -- the one whose folder it goes
    /// in (`files::channel_dir`), as its stored name: the active channel if
    /// its transcript carries the file, else the first joined channel that
    /// does. A file shared in several channels is saved to the one you are
    /// looking at when it can be; `/save <prefix>` can also name a file from
    /// a channel you aren't in, and then it goes where it was shared.
    ///
    /// # Panics
    /// Panics if no joined channel carries `hash` -- the callers have just
    /// found the file in a transcript (`resolve_attachment`,
    /// `newest_shared_file`).
    fn channel_of_file(&self, hash: Hash) -> &str {
        let active = self.active().name.as_str();
        self.channels
            .iter()
            .map(|channel| channel.name.as_str())
            .filter(|channel| {
                self.shared_files
                    .get(*channel)
                    .is_some_and(|files| files.iter().any(|shared| shared.file.hash == hash))
            })
            .find(|channel| *channel == active)
            .or_else(|| {
                self.channels
                    .iter()
                    .map(|channel| channel.name.as_str())
                    .find(|channel| {
                        self.shared_files.get(*channel).is_some_and(|files| {
                            files.iter().any(|shared| shared.file.hash == hash)
                        })
                    })
            })
            .expect("a file /save resolved was found in shared-file metadata")
    }

    /// Who `/save` asks for the file `hash`, best bet first: its `sender`;
    /// then the peers online right now in the channels that carry it
    /// (`Channel::peers`); then everyone who has posted in those channels.
    /// Anyone who saved the file serves it from their own blob store (see
    /// `BackfillStore::fetch_file`), so a file does not depend on its
    /// sender being online -- but nothing says who saved it, so every peer
    /// we know of is a candidate, the ones reachable now before the ones
    /// that are probably gone. Posters come after the online peers because
    /// gossip only keeps a handful of direct neighbors: someone who has the
    /// file may be online without being one of them.
    ///
    /// Never this device -- except as the sender, when the file is already
    /// in our own blob store and needs no one -- and each peer once. A
    /// channel carries the file if its loaded transcript has a message with
    /// this hash; a file shared in several asks all of them, so the answer
    /// does not depend on which channel `/save` happened to resolve it in.
    fn providers_for(&self, hash: Hash, sender: [u8; 32]) -> Vec<[u8; 32]> {
        let channels: Vec<&Channel> = self
            .channels
            .iter()
            .filter(|channel| {
                self.shared_files
                    .get(&channel.name)
                    .is_some_and(|files| files.iter().any(|shared| shared.file.hash == hash))
            })
            .collect();
        let online = channels
            .iter()
            .flat_map(|channel| channel.peers.iter().copied());
        let posters = channels.iter().flat_map(|channel| {
            self.shared_files
                .get(&channel.name)
                .into_iter()
                .flatten()
                .map(|shared| shared.sender)
        });
        let live_posters = channels.iter().flat_map(|channel| {
            channel.messages.iter().filter_map(|line| match line {
                TranscriptLine::Chat(message) => Some(message.sender),
                TranscriptLine::System(_) => None,
            })
        });

        let mut providers = vec![sender];
        for id in online.chain(posters).chain(live_posters) {
            if id != self.self_id && !providers.contains(&id) {
                providers.push(id);
            }
        }
        providers
    }

    /// Whether keys -- and pastes -- currently go to the message box. False
    /// while a live pairing redeem is awaiting confirmation, a reply or
    /// thread pick is in progress, or the `Ctrl+P` switcher, the file picker
    /// or the files overlay is open: each takes over input ahead of normal
    /// typing (see `handle_key`). main.rs only treats a pasted file path as
    /// a dropped file while this is true, since staging files while the user
    /// is in the middle of something else would put them somewhere they are
    /// not looking.
    pub fn is_composing(&self) -> bool {
        self.pending_redeem.is_none()
            && self.picking_reply.is_none()
            && self.picking_thread.is_none()
            && self.switching.is_none()
            && self.command_picker.is_none()
            && self.viewing_peers.is_none()
            && self.picking_file.is_none()
            && self.viewing_files.is_none()
    }

    /// Inserts a terminal bracketed-paste's full text at the cursor as one
    /// atomic edit (see main.rs's `Event::Paste` handling, enabled via
    /// `EnableBracketedPaste`). Without bracketed paste, a multi-line
    /// paste instead arrives as a fast burst of ordinary key events, so an
    /// embedded newline mid-paste would hit `submit_input` partway through
    /// -- prematurely sending a partial line or running a stray
    /// `/command`. `paste_text` never submits, so a pasted newline just
    /// becomes a real line break (`insert_newline`) instead. A paste that is
    /// really a dropped file never gets here -- main.rs's `handle_paste`
    /// diverts it first.
    pub fn paste_text(&mut self, text: &str) {
        // An open switcher owns input: a paste filters it rather than
        // landing, unseen, in the draft underneath. Its query is one line,
        // so line breaks and other control characters are dropped.
        if self.switching.is_some() {
            self.edit_switcher_query(|query| {
                query.extend(text.chars().filter(|c| !c.is_control()));
            });
            return;
        }
        if self.command_picker.is_some() {
            self.edit_command_picker_query(|query| {
                query.extend(text.chars().filter(|c| !c.is_control()));
            });
            return;
        }
        if self.viewing_peers.is_some() {
            return;
        }
        // So does an open file picker, whose filter is one line too.
        if let Some(picker) = &mut self.picking_file {
            picker.edit_query(|query| {
                query.extend(text.chars().filter(|c| !c.is_control()));
            });
            return;
        }
        // And an open files overlay, with the same one-line filter.
        if self.viewing_files.is_some() {
            self.edit_files_query(|query| {
                query.extend(text.chars().filter(|c| !c.is_control()));
            });
            return;
        }
        let channel = self.active_mut();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                // `\r\n` counts as a single line break, not two -- without
                // this, a Windows-style multi-line paste would insert an
                // extra blank line between paragraphs.
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    channel.insert_newline();
                }
                '\n' => channel.insert_newline(),
                c => channel.insert_char(c),
            }
        }
    }
}

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
mod tests {
    use std::net::SocketAddr;

    use super::*;
    use crate::message::{DeviceCert, IdentityAnnounce, VersionAnnounce};
    use iroh::{EndpointAddr, SecretKey, TransportAddr};

    const SELF_ID: [u8; 32] = [9; 32];
    const PEER_ID: [u8; 32] = [7; 32];
    const PEER_ID_2: [u8; 32] = [8; 32];
    const PEER_ID_3: [u8; 32] = [6; 32];

    fn app() -> AppState {
        AppState::new(SELF_ID, vec!["general".to_string()], "general")
    }

    fn multi_channel_app() -> AppState {
        AppState::new(
            SELF_ID,
            vec!["general".to_string(), "random".to_string()],
            "random",
        )
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Unwraps a `TranscriptLine::Chat`'s inner message, panicking with a
    /// helpful message if the line is actually a `TranscriptLine::System`.
    fn as_chat(line: &TranscriptLine) -> &ChatMessage {
        match line {
            TranscriptLine::Chat(message) => message,
            TranscriptLine::System(text) => {
                panic!("expected a chat message, got system line: {text:?}")
            }
        }
    }

    /// Unwraps a `TranscriptLine::System`'s inner text, panicking if the
    /// line is actually a `TranscriptLine::Chat`.
    fn as_system(line: &TranscriptLine) -> &str {
        match line {
            TranscriptLine::System(text) => text,
            TranscriptLine::Chat(_) => panic!("expected a system line, got a chat message"),
        }
    }

    /// Types `text` character-by-character then presses `Enter`, returning
    /// whatever action (if any) that produced -- shared by the `/alias`
    /// tests below, which all follow this shape.
    fn submit(app: &mut AppState, text: &str) -> Option<InputAction> {
        for c in text.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Enter))
    }

    #[test]
    fn typing_composes_input() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        assert_eq!(app.active().input, "hi");
    }

    #[test]
    fn backspace_edits_input() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.active().input, "h");
    }

    #[test]
    fn paste_text_inserts_the_whole_string_at_once() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.paste_text("i there");
        assert_eq!(app.active().input, "hi there");
        assert_eq!(app.active().cursor, "hi there".chars().count());
    }

    #[test]
    fn paste_text_inserts_embedded_newlines_literally() {
        // Without bracketed paste, a multi-line paste's embedded `Enter`
        // keystrokes would submit partway through -- `paste_text` is the
        // atomic alternative, so it can safely turn a pasted newline into
        // a real line break instead of flattening it to a space.
        let mut app = app();
        app.paste_text("line one\nline two\r\nline three");
        assert_eq!(app.active().input, "line one\nline two\nline three");
    }

    #[test]
    fn paste_text_does_not_submit_even_when_it_contains_a_slash_command() {
        let mut app = app();
        let before = app.active().messages.len();
        app.paste_text("/join sneaky\nrest of paste");
        assert_eq!(app.active().messages.len(), before, "paste must not submit");
        assert_eq!(app.active().input, "/join sneaky\nrest of paste");
    }

    #[test]
    fn alt_enter_inserts_a_newline_instead_of_submitting() {
        let mut app = app();
        let before = app.active().messages.len();
        app.handle_key(key(KeyCode::Char('h')));
        let action = app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        app.handle_key(key(KeyCode::Char('i')));
        assert!(action.is_none());
        assert_eq!(app.active().input, "h\ni");
        assert_eq!(app.active().messages.len(), before, "must not submit");
    }

    #[test]
    fn ctrl_j_inserts_a_newline_instead_of_submitting() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        let action = app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
        assert!(action.is_none());
        assert_eq!(app.active().input, "h\n");
    }

    #[test]
    fn plain_enter_still_submits_a_multiline_message() {
        let mut app = app();
        for c in "line one".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
        for c in "line two".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let sent = app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.active().input, "");
        match sent.expect("enter with non-empty input returns an action") {
            InputAction::Send(_, message) => assert_eq!(message.text, "line one\nline two"),
            _ => panic!("expected InputAction::Send"),
        }
    }

    #[test]
    fn home_and_end_move_within_the_current_line_only() {
        let mut app = app();
        app.paste_text("first\nsecond"); // cursor ends on the second line
        app.handle_key(key(KeyCode::Home));
        assert_eq!(
            app.active().cursor,
            "first\n".chars().count(),
            "Home lands at the start of the current line, not the whole buffer"
        );
        app.handle_key(key(KeyCode::End));
        assert_eq!(
            app.active().cursor,
            app.active().input.chars().count(),
            "End lands at the end of the current line"
        );
    }

    #[test]
    fn ctrl_k_keeps_a_multiline_draft_unchanged_while_opening_the_picker() {
        let mut app = app();
        app.paste_text("first\nsecond");
        app.handle_key(key(KeyCode::Home)); // start of "second"
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(
            app.active().input,
            "first\nsecond",
            "opening the picker must not change the draft"
        );
        assert!(app.command_picker.is_some());
    }

    #[test]
    fn ctrl_u_deletes_only_to_the_start_of_the_current_line() {
        let mut app = app();
        app.paste_text("first\nsecond");
        app.handle_key(key(KeyCode::End)); // already at the end of "second"
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(
            app.active().input,
            "first\n",
            "Ctrl+U must not touch the previous line"
        );
    }

    #[test]
    fn enter_sends_and_clears_input() {
        let mut app = app();
        let before = app.active().messages.len();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        let sent = app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.active().input, "");
        assert_eq!(app.active().messages.len(), before + 1);
        let last = as_chat(app.active().messages.back().unwrap());
        assert_eq!(last.text, "hi");
        assert_eq!(last.sender, SELF_ID);
        match sent.expect("enter with non-empty input returns an action") {
            InputAction::Send(channel, message) => {
                assert_eq!(channel, "general");
                assert_eq!(message.text, "hi");
                assert_eq!(message.sender, SELF_ID);
            }
            _ => panic!("expected InputAction::Send"),
        }
    }

    #[test]
    fn empty_enter_does_not_send() {
        let mut app = app();
        let before = app.active().messages.len();
        let sent = app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.active().messages.len(), before);
        assert!(sent.is_none());
    }

    #[test]
    fn esc_quits() {
        let mut app = app();
        app.handle_key(key(KeyCode::Esc));
        assert!(app.should_quit);
    }
    #[test]
    fn esc_clears_an_active_search_before_other_normal_escape_actions() {
        let mut app = app();
        let matching = compose_message(PEER_ID, "find me", None);
        let matching_id = matching.id;
        let non_matching = compose_message(PEER_ID, "leave me", None);
        let non_matching_id = non_matching.id;
        app.handle_net_event(NetEvent::Received("general".to_string(), matching));
        app.handle_net_event(NetEvent::Received("general".to_string(), non_matching));
        app.stage_attachment("general", staged("map.png"));
        submit(&mut app, "/search find");
        assert_eq!(app.active().visible_chat_ids(), vec![matching_id]);

        app.handle_key(key(KeyCode::Esc));

        assert!(app.active().search.is_none());
        assert_eq!(
            app.active().visible_chat_ids(),
            vec![matching_id, non_matching_id],
            "clearing the filter restores the unfiltered transcript"
        );
        assert_eq!(app.active().staged, vec![staged("map.png")]);
        assert!(!app.should_quit);
        assert_eq!(
            as_system(app.active().messages.back().unwrap()),
            "search cleared"
        );
    }

    #[test]
    fn ctrl_c_quits() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn display_name_maps_self_and_falls_back_to_hex_for_others() {
        let app = app();
        assert_eq!(app.display_name(&SELF_ID), "you");
        assert_eq!(app.display_name(&PEER_ID), hex_prefix(&PEER_ID));
    }

    #[test]
    fn display_name_uses_petname_when_one_is_set() {
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        assert_eq!(app.display_name(&PEER_ID), "Alice");
    }

    #[test]
    fn display_name_uses_nickname_when_no_petname_is_set() {
        let mut app = app();
        let result = app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "Alice".to_string(),
            },
        ));
        assert!(result.is_none());
        assert_eq!(
            app.display_name(&PEER_ID),
            format!("Alice ({})", hex_prefix(&PEER_ID)),
            "a broadcast nickname must be shown alongside the hex id, since it's spoofable"
        );
    }

    #[test]
    fn display_name_prefers_petname_over_nickname() {
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Bob".to_string())]));
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "Alice".to_string(),
            },
        ));
        assert_eq!(
            app.display_name(&PEER_ID),
            "Bob",
            "a pinned local petname must win over a peer's own broadcast nickname"
        );
    }

    #[test]
    fn identity_event_overwrites_previous_nickname_for_the_same_sender() {
        let mut app = app();
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "Alice".to_string(),
            },
        ));
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "Alicia".to_string(),
            },
        ));
        assert_eq!(
            app.display_name(&PEER_ID),
            format!("Alicia ({})", hex_prefix(&PEER_ID)),
            "display_name must reflect the last-seen nickname, not the first"
        );
    }

    fn device_cert(user_key: &SecretKey, device_id: [u8; 32]) -> DeviceCert {
        DeviceCert::new(device_id, user_key, 1_000)
    }

    #[test]
    fn canonical_id_is_unchanged_when_no_cert_is_known() {
        let app = app();
        assert_eq!(app.canonical_id(&PEER_ID), PEER_ID);
    }

    #[test]
    fn a_peer_with_no_certificate_displays_exactly_as_before() {
        let app = app();
        assert_eq!(app.display_name(&PEER_ID), hex_prefix(&PEER_ID));
    }

    #[test]
    fn device_event_makes_canonical_id_resolve_to_the_user_id() {
        let mut app = app();
        let user_key = SecretKey::generate();
        let user_id = *user_key.public().as_bytes();
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        assert_eq!(app.canonical_id(&PEER_ID), user_id);
    }

    #[test]
    fn two_certified_devices_share_a_display_name() {
        let mut app = app();
        let user_key = SecretKey::generate();
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID_2),
        ));
        app.load_contacts(HashMap::from([(
            app.canonical_id(&PEER_ID),
            "Jurgen".to_string(),
        )]));

        assert_eq!(app.display_name(&PEER_ID), "Jurgen");
        assert_eq!(app.display_name(&PEER_ID_2), "Jurgen");
    }

    #[test]
    fn unique_peer_ids_collapses_two_certified_devices_into_one() {
        let mut app = app();
        let user_key = SecretKey::generate();
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID_2),
        ));

        let unique = app.unique_peer_ids(&[PEER_ID, PEER_ID_2]);

        assert_eq!(
            unique,
            vec![PEER_ID],
            "two devices certified under the same user key must collapse to one"
        );
    }

    #[test]
    fn unique_peer_ids_keeps_uncertified_peers_distinct() {
        let app = app();
        let unique = app.unique_peer_ids(&[PEER_ID, PEER_ID_2]);
        assert_eq!(
            unique,
            vec![PEER_ID, PEER_ID_2],
            "peers with no known certificate must never be merged"
        );
    }

    #[test]
    fn unique_peer_ids_preserves_original_order_including_mixed_groups() {
        let mut app = app();
        let user_key = SecretKey::generate();
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID_2),
        ));

        // PEER_ID and PEER_ID_2 are the same person's two devices;
        // PEER_ID_3 has no certificate at all.
        let unique = app.unique_peer_ids(&[PEER_ID, PEER_ID_3, PEER_ID_2]);

        assert_eq!(unique, vec![PEER_ID, PEER_ID_3]);
    }

    #[test]
    fn display_name_shows_you_for_a_peer_certified_under_our_own_user_key() {
        let mut app = app();
        let user_key = SecretKey::generate();
        let user_id = *user_key.public().as_bytes();
        app.load_self_user_id(user_id);
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));

        assert_eq!(
            app.display_name(&PEER_ID),
            "you",
            "a paired sibling device is us, not a stranger"
        );
    }

    #[test]
    fn display_name_you_check_for_a_sibling_device_overrides_any_petname() {
        let mut app = app();
        let user_key = SecretKey::generate();
        let user_id = *user_key.public().as_bytes();
        app.load_self_user_id(user_id);
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        app.load_contacts(HashMap::from([(user_id, "My Phone".to_string())]));

        assert_eq!(
            app.display_name(&PEER_ID),
            "you",
            "recognizing our own identity must win over a petname, mirroring the self_id check above it"
        );
    }

    #[test]
    fn unique_peer_ids_excludes_a_sibling_device_entirely() {
        let mut app = app();
        let user_key = SecretKey::generate();
        let user_id = *user_key.public().as_bytes();
        app.load_self_user_id(user_id);
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));

        let unique = app.unique_peer_ids(&[PEER_ID, PEER_ID_2]);

        assert_eq!(
            unique,
            vec![PEER_ID_2],
            "our own sibling device is already shown as the sidebar's separate 'you' row, \
             so it must not also appear as a peer"
        );
    }

    #[test]
    fn unique_peer_ids_is_empty_when_the_only_peer_is_a_sibling_device() {
        let mut app = app();
        let user_key = SecretKey::generate();
        let user_id = *user_key.public().as_bytes();
        app.load_self_user_id(user_id);
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));

        assert!(app.unique_peer_ids(&[PEER_ID]).is_empty());
    }

    #[test]
    fn peer_presence_keeps_a_left_peer_with_its_last_seen_time() {
        let mut app = app();
        app.record_peer_seen("general", PEER_ID, 100);
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));

        assert_eq!(
            app.peer_presence(),
            vec![PeerPresence {
                id: PEER_ID,
                last_seen_unix_ms: None,
            }]
        );

        app.handle_net_event(NetEvent::PeerLeft("general".to_string(), PEER_ID));

        assert_eq!(
            app.peer_presence(),
            vec![PeerPresence {
                id: PEER_ID,
                last_seen_unix_ms: Some(100),
            }]
        );
    }

    #[test]
    fn hovered_peer_reports_alias_nickname_presence_and_version() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert!(app.set_peer_hover(Some(PEER_ID)));
        assert!(!app.set_peer_hover(Some(PEER_ID)));
        assert_eq!(
            app.hovered_peer(),
            Some(PeerDetails {
                id: PEER_ID,
                alias: None,
                nickname: None,
                last_seen_unix_ms: None,
                version: None,
            })
        );

        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "ally".to_string(),
            },
        ));
        app.handle_net_event(NetEvent::Version(
            "general".to_string(),
            VersionAnnounce {
                sender: PEER_ID,
                version: "1.2.3".to_string(),
                git_hash: "abc1234".to_string(),
            },
        ));
        app.record_peer_seen("general", PEER_ID, 100);
        app.handle_net_event(NetEvent::PeerLeft("general".to_string(), PEER_ID));

        assert_eq!(
            app.hovered_peer(),
            Some(PeerDetails {
                id: PEER_ID,
                alias: Some("Alice".to_string()),
                nickname: Some("ally".to_string()),
                last_seen_unix_ms: Some(100),
                version: Some("1.2.3".to_string()),
            })
        );
    }

    #[test]
    fn hovered_peer_is_hidden_for_an_unlisted_peer_and_behind_overlays() {
        let mut app = app();
        app.set_peer_hover(Some(PEER_ID));
        assert_eq!(app.hovered_peer(), None, "not in the active channel");

        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert!(app.hovered_peer().is_some());
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert_eq!(app.hovered_peer(), None, "the command picker owns input");
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

        submit(&mut app, "/hints");
        assert!(app.peer_hover.is_none());
        assert!(!app.set_peer_hover(Some(PEER_ID)));
    }

    #[test]
    fn peer_presence_loads_last_seen_per_channel() {
        let mut app = multi_channel_app();
        app.load_last_seen(HashMap::from([
            ("general".to_string(), HashMap::from([(PEER_ID, 100)])),
            ("random".to_string(), HashMap::from([(PEER_ID_2, 200)])),
        ]));

        assert_eq!(
            app.peer_presence(),
            vec![PeerPresence {
                id: PEER_ID_2,
                last_seen_unix_ms: Some(200),
            }]
        );
        app.active = 0;
        assert_eq!(
            app.peer_presence(),
            vec![PeerPresence {
                id: PEER_ID,
                last_seen_unix_ms: Some(100),
            }]
        );
    }

    #[test]
    fn peer_presence_prefers_live_presence_and_deduplicates_certified_devices() {
        let mut app = app();
        let user_key = SecretKey::generate();
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID_2),
        ));
        app.record_peer_seen("general", PEER_ID, 100);
        app.record_peer_seen("general", PEER_ID_2, 200);
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));

        assert_eq!(
            app.peer_presence(),
            vec![PeerPresence {
                id: PEER_ID,
                last_seen_unix_ms: None,
            }]
        );

        app.handle_net_event(NetEvent::PeerLeft("general".to_string(), PEER_ID));
        assert_eq!(
            app.peer_presence(),
            vec![PeerPresence {
                id: PEER_ID_2,
                last_seen_unix_ms: Some(200),
            }]
        );
    }

    #[test]
    fn device_event_migrates_a_nickname_seen_before_the_certificate() {
        let mut app = app();
        let user_key = SecretKey::generate();
        let user_id = *user_key.public().as_bytes();
        // The nickname arrives first -- gossip has no delivery ordering
        // guarantee, so this must not get orphaned once the cert arrives.
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "alice".to_string(),
            },
        ));
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));

        assert_eq!(
            app.display_name(&PEER_ID),
            format!("alice ({})", hex_prefix(&PEER_ID))
        );
        assert_eq!(
            app.nicknames.get(&user_id).map(String::as_str),
            Some("alice"),
            "the nickname must be re-keyed onto the canonical user id"
        );
    }

    #[test]
    fn alias_assigns_a_petname_for_a_currently_online_peer() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));

        let full_id = hex_id(&PEER_ID);
        let action = submit(&mut app, &format!("/alias {} Alice", &full_id[..8]));

        match action.expect("a successful /alias returns an action") {
            InputAction::Alias(id, name) => {
                assert_eq!(id, PEER_ID);
                assert_eq!(name, "Alice");
            }
            _ => panic!("expected InputAction::Alias"),
        }
        assert_eq!(app.display_name(&PEER_ID), "Alice");
    }

    #[test]
    fn alias_matches_a_peer_known_only_from_a_past_message() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hi", None),
        ));

        let full_id = hex_id(&PEER_ID);
        let action = submit(&mut app, &format!("/alias {} Bob", &full_id[..8]));

        assert!(
            matches!(action, Some(InputAction::Alias(id, _)) if id == PEER_ID),
            "a peer seen only as a message sender must still be aliasable"
        );
    }

    #[test]
    fn alias_reports_an_error_for_an_unknown_prefix() {
        let mut app = app();
        let action = submit(&mut app, "/alias ffffffff Carol");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("no known peer"), "got: {last}");
    }

    #[test]
    fn alias_reports_an_error_for_an_ambiguous_prefix() {
        let mut app = app();
        let mut peer_a = [0xAB; 32];
        let mut peer_b = [0xAB; 32];
        peer_a[4] = 0x01;
        peer_b[4] = 0x02;
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), peer_a));
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), peer_b));

        // Both ids share this 4-byte (8 hex char) prefix.
        let full_id = hex_id(&peer_a);
        let action = submit(&mut app, &format!("/alias {} Dave", &full_id[..8]));

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("multiple peers"), "got: {last}");
    }

    #[test]
    fn alias_rejects_a_non_hex_prefix() {
        let mut app = app();
        let action = submit(&mut app, "/alias not-hex Eve");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("invalid hex prefix"), "got: {last}");
    }

    #[test]
    fn alias_usage_error_when_name_is_missing() {
        let mut app = app();
        let action = submit(&mut app, "/alias ffffffff");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"));
    }

    #[test]
    fn alias_applies_to_a_second_device_once_its_certificate_is_known() {
        let mut app = app();
        let user_key = SecretKey::generate();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID_2),
        ));

        let full_id = hex_id(&PEER_ID);
        submit(&mut app, &format!("/alias {} Jurgen", &full_id[..8]));

        assert_eq!(app.display_name(&PEER_ID), "Jurgen");
        assert_eq!(
            app.display_name(&PEER_ID_2),
            "Jurgen",
            "aliasing one of a person's devices must apply to the other once both certs are known"
        );
    }

    #[test]
    fn forgetting_a_device_stops_it_from_being_grouped_under_its_canonical_id() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        let user_key = SecretKey::generate();
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        assert_ne!(
            app.canonical_id(&PEER_ID),
            PEER_ID,
            "sanity check: cert took effect"
        );

        submit(
            &mut app,
            &format!("/forget-device {}", &hex_id(&PEER_ID)[..8]),
        );

        assert_eq!(
            app.canonical_id(&PEER_ID),
            PEER_ID,
            "a forgotten device must resolve to itself, ignoring any known certificate"
        );
    }

    #[test]
    fn forget_device_returns_the_action_with_the_resolved_id() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        let full_id = hex_id(&PEER_ID);

        let action = submit(&mut app, &format!("/forget-device {}", &full_id[..8]));

        match action.expect("a successful /forget-device returns an action") {
            InputAction::ForgetDevice(id) => assert_eq!(id, PEER_ID),
            _ => panic!("expected InputAction::ForgetDevice"),
        }
    }

    #[test]
    fn forget_device_reports_an_error_for_an_unknown_prefix() {
        let mut app = app();
        let action = submit(&mut app, "/forget-device ffffffff");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("no known peer"), "got: {last}");
    }

    #[test]
    fn nick_command_returns_action_and_pushes_system_notice() {
        let mut app = app();
        let action = submit(&mut app, "/nick Alice");
        match action.expect("a successful /nick returns an action") {
            InputAction::Nick(name) => assert_eq!(name, "Alice"),
            _ => panic!("expected InputAction::Nick"),
        }
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("Alice"), "got: {last}");
    }

    #[test]
    fn nick_usage_error_when_name_is_missing() {
        let mut app = app();
        let action = submit(&mut app, "/nick");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"));
    }

    #[test]
    fn pair_with_no_argument_returns_the_pair_action() {
        let mut app = app();
        let action = submit(&mut app, "/pair");
        assert!(matches!(action, Some(InputAction::Pair)));
    }

    fn sample_pairing_ticket() -> String {
        let addr = EndpointAddr::from_parts(
            SecretKey::generate().public(),
            [TransportAddr::Ip(SocketAddr::from(([127, 0, 0, 1], 4242)))],
        );
        PairingTicket {
            addr,
            secret: rand::random(),
        }
        .encode_string()
    }

    #[test]
    fn pair_with_a_valid_ticket_warns_instead_of_acting_immediately() {
        let mut app = app();
        let ticket = sample_pairing_ticket();
        let action = submit(&mut app, &format!("/pair {ticket}"));
        assert!(action.is_none(), "must not act until explicitly confirmed");
        assert!(!app.should_quit);
        assert!(app.restart_with_ticket.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("ERASE"), "got: {last}");
        assert!(
            last.contains("1 channel(s)"),
            "should name the current channel count, got: {last}"
        );
    }

    #[test]
    fn pair_with_a_malformed_ticket_reports_an_error_and_never_arms_confirmation() {
        let mut app = app();
        let action = submit(&mut app, "/pair not-a-real-ticket");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("invalid pairing ticket"), "got: {last}");

        // Confirmation must not have been armed -- a normal message typed
        // right after still sends normally instead of being swallowed as
        // a confirm/cancel keypress.
        let sent = submit(&mut app, "hello");
        assert!(matches!(sent, Some(InputAction::Send(..))));
    }

    #[test]
    fn confirming_a_pending_pair_redeem_arms_restart_and_quits() {
        let mut app = app();
        let ticket = sample_pairing_ticket();
        submit(&mut app, &format!("/pair {ticket}"));

        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.restart_with_ticket, Some(ticket));
        assert!(app.should_quit);
    }

    #[test]
    fn canceling_a_pending_pair_redeem_clears_it_without_quitting() {
        let mut app = app();
        let ticket = sample_pairing_ticket();
        submit(&mut app, &format!("/pair {ticket}"));

        app.handle_key(key(KeyCode::Esc));

        assert!(!app.should_quit);
        assert!(app.restart_with_ticket.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "pairing cancelled");

        // No longer armed -- typing now reaches the input box normally
        // instead of being swallowed as a confirm/cancel keypress.
        app.handle_key(key(KeyCode::Char('h')));
        assert_eq!(app.active().input, "h");
    }

    #[test]
    fn unrelated_keys_are_ignored_while_a_pair_redeem_is_pending() {
        let mut app = app();
        let ticket = sample_pairing_ticket();
        submit(&mut app, &format!("/pair {ticket}"));

        app.handle_key(key(KeyCode::Char('y')));

        assert!(!app.should_quit);
        assert!(app.restart_with_ticket.is_none());
        assert_eq!(
            app.active().input,
            "",
            "a stray keystroke must not leak into the input box while armed"
        );
    }

    #[test]
    fn newer_peer_version_pushes_a_notice_once() {
        let mut app = app();
        let announce = VersionAnnounce {
            sender: PEER_ID,
            version: "999.0.0".to_string(),
            git_hash: "abc1234".to_string(),
        };
        app.handle_net_event(NetEvent::Version("general".to_string(), announce.clone()));
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("999.0.0"), "got: {last}");

        // Re-announcing the exact same version (e.g. on a reconnect) must
        // not repeat the notice.
        let before = app.active().messages.len();
        app.handle_net_event(NetEvent::Version("general".to_string(), announce));
        assert_eq!(
            app.active().messages.len(),
            before,
            "must not repeat the notice for an unchanged re-announce"
        );
    }

    #[test]
    fn older_peer_version_does_not_notice_but_is_recorded_for_peer_details() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        let before = app.active().messages.len();

        app.handle_net_event(NetEvent::Version(
            "general".to_string(),
            VersionAnnounce {
                sender: PEER_ID,
                version: "0.0.0".to_string(),
                git_hash: "abc1234".to_string(),
            },
        ));
        assert_eq!(
            app.active().messages.len(),
            before,
            "an older version must not push a notice"
        );

        submit(&mut app, "/who");
        assert_eq!(
            app.viewing_peers
                .as_ref()
                .unwrap()
                .peers
                .first()
                .unwrap()
                .version
                .as_deref(),
            Some("0.0.0")
        );
    }

    #[test]
    fn version_command_reports_our_own_build() {
        let mut app = app();
        let action = submit(&mut app, "/version");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains(version::VERSION), "got: {last}");
    }

    #[test]
    fn version_command_also_reports_a_known_newer_peer_version() {
        let mut app = app();
        app.handle_net_event(NetEvent::Version(
            "general".to_string(),
            VersionAnnounce {
                sender: PEER_ID,
                version: "999.0.0".to_string(),
                git_hash: "abc1234".to_string(),
            },
        ));
        submit(&mut app, "/version");
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("999.0.0"), "got: {last}");
    }

    #[test]
    fn left_right_move_cursor_without_editing() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        assert_eq!(app.active().cursor, 2);
        app.handle_key(key(KeyCode::Left));
        assert_eq!(app.active().cursor, 1);
        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.active().cursor, 2);
        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.active().cursor, 2);
        assert_eq!(app.active().input, "hi");
    }

    #[test]
    fn insert_in_the_middle_of_input() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(key(KeyCode::Left));
        app.handle_key(key(KeyCode::Char('X')));
        assert_eq!(app.active().input, "hXi");
        assert_eq!(app.active().cursor, 2);
    }

    #[test]
    fn home_and_end_jump_cursor() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(key(KeyCode::Home));
        assert_eq!(app.active().cursor, 0);
        app.handle_key(key(KeyCode::End));
        assert_eq!(app.active().cursor, 2);
    }

    #[test]
    fn ctrl_a_and_ctrl_e_jump_cursor() {
        let mut app = app();
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));
        app.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        assert_eq!(app.active().cursor, 0);
        app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert_eq!(app.active().cursor, 2);
    }

    #[test]
    fn delete_key_removes_char_at_cursor() {
        let mut app = app();
        for c in ['h', 'i', '!'] {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Home));
        app.handle_key(key(KeyCode::Delete));
        assert_eq!(app.active().input, "i!");
        assert_eq!(app.active().cursor, 0);
    }

    #[test]
    fn ctrl_u_deletes_to_start() {
        let mut app = app();
        for c in ['h', 'i', '!'] {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Left));
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert_eq!(app.active().input, "!");
        assert_eq!(app.active().cursor, 0);
    }

    #[test]
    fn ctrl_k_opens_the_command_picker_without_changing_the_draft() {
        let mut app = app();
        for c in ['h', 'i', '!'] {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert!(app.command_picker.is_some());
        assert_eq!(app.active().input, "hi!");
        assert!(!app.is_composing());
    }
    #[test]
    fn command_catalog_exposes_command_shortcuts() {
        let shortcuts: Vec<_> = command_catalog()
            .iter()
            .filter_map(|command| command.shortcut.map(|shortcut| (command.syntax, shortcut)))
            .collect();
        assert_eq!(
            shortcuts,
            vec![
                ("/paste", "Ctrl+V"),
                ("/reply [text]", "Ctrl+R"),
                ("/thread", "Ctrl+T"),
            ]
        );
    }

    #[test]
    fn command_picker_prioritizes_syntax_and_alias_matches_over_descriptions() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        type_text(&mut app, "file");
        assert_eq!(
            app.command_picker_matches().first(),
            Some(&21),
            "/files must precede commands that only mention files in their descriptions"
        );

        for _ in 0..4 {
            app.handle_key(key(KeyCode::Backspace));
        }
        type_text(&mut app, "/s");
        assert!(app.command_picker_matches().contains(&23));
    }

    #[test]
    fn command_picker_enter_executes_the_selected_command_and_preserves_the_draft() {
        let mut app = app();
        type_text(&mut app, "old draft");
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        type_text(&mut app, "invite");

        let action = app.handle_key(key(KeyCode::Enter));
        assert!(matches!(action, Some(InputAction::Invite(channel)) if channel == "general"));
        assert!(app.command_picker.is_none());
        assert_eq!(app.active().input, "old draft");
        assert_eq!(app.active().cursor, 9);
    }

    #[test]
    fn ctrl_w_deletes_word_backward() {
        let mut app = app();
        for c in "hello world".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.active().input, "hello ");
        assert_eq!(app.active().cursor, 6);
        app.handle_key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
        assert_eq!(app.active().input, "");
        assert_eq!(app.active().cursor, 0);
    }

    #[test]
    fn peer_joined_and_left_update_peers() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert_eq!(app.active().peers, vec![PEER_ID]);
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert_eq!(
            app.active().peers,
            vec![PEER_ID],
            "joining twice should not duplicate"
        );
        app.handle_net_event(NetEvent::PeerLeft("general".to_string(), PEER_ID));
        assert!(app.active().peers.is_empty());
    }

    #[test]
    fn received_message_is_deduped_by_id() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hi from a peer", None);
        let before = app.active().messages.len();
        app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
        assert_eq!(app.active().messages.len(), before + 1);
        // Gossip is best-effort and can redeliver -- the same id must not
        // be shown twice.
        app.handle_net_event(NetEvent::Received("general".to_string(), message));
        assert_eq!(app.active().messages.len(), before + 1);
    }

    #[test]
    fn a_message_is_still_deduped_after_hundreds_of_others_arrive() {
        // Regression test for a real duplication bug: `seen_ids` used to be
        // a 256-entry recency window, but `net::Net::sync_history` can
        // redeliver a channel's *entire* history at any time (not just
        // messages sent since the last sync -- see
        // `backfill::BackfillStore::fetch`), so an old-but-still-displayed
        // message must still be rejected as a duplicate no matter how much
        // channel activity happened after it.
        let mut app = app();
        let first = compose_message(PEER_ID, "the first message", None);
        app.handle_net_event(NetEvent::Received("general".to_string(), first.clone()));

        for i in 0..300 {
            app.handle_net_event(NetEvent::Received(
                "general".to_string(),
                compose_message(PEER_ID, &format!("filler {i}"), None),
            ));
        }

        let before = app.active().messages.len();
        let result = app.handle_net_event(NetEvent::Received("general".to_string(), first));
        assert!(
            result.is_none(),
            "a redelivery of an old message must still be rejected as a duplicate"
        );
        assert_eq!(
            app.active().messages.len(),
            before,
            "must not be re-inserted into the transcript"
        );
    }

    #[test]
    fn tab_and_backtab_cycle_channels_and_clear_unread() {
        let mut app = multi_channel_app();
        assert_eq!(app.active, 1, "starts on the last-joined channel");

        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.active, 0, "Tab wraps around");
        app.handle_key(key(KeyCode::Tab));
        assert_eq!(app.active, 1);

        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.active, 0, "Shift+Tab moves backward");
        app.handle_key(key(KeyCode::BackTab));
        assert_eq!(app.active, 1, "and wraps the other way");
    }

    #[test]
    fn switching_channels_preserves_each_channels_own_draft() {
        // Regression test for features.md's "Per-channel input drafts":
        // `input`/`cursor` live on `Channel`, not `AppState`, so switching
        // tabs must neither leak a half-typed draft into the newly active
        // channel nor lose it from the one it was written for.
        let mut app = multi_channel_app();
        assert_eq!(app.active().name, "random");

        for c in "hello from random".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        app.handle_key(key(KeyCode::Left));
        app.handle_key(key(KeyCode::Left));
        let cursor_in_random = app.active().cursor;

        app.handle_key(key(KeyCode::Tab)); // wraps to "general"
        assert_eq!(app.active().name, "general");
        assert_eq!(
            app.active().input,
            "",
            "switching tabs must not leak the other channel's draft in"
        );
        assert_eq!(app.active().cursor, 0);

        for c in "typed in general".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }

        app.handle_key(key(KeyCode::Tab)); // back to "random"
        assert_eq!(app.active().name, "random");
        assert_eq!(
            app.active().input,
            "hello from random",
            "switching away and back must not lose or clear the draft"
        );
        assert_eq!(app.active().cursor, cursor_in_random);

        app.handle_key(key(KeyCode::BackTab)); // back to "general"
        assert_eq!(app.active().input, "typed in general");
    }

    #[test]
    fn switching_into_a_channel_clears_its_unread_count_and_sets_an_anchor() {
        let mut app = multi_channel_app();
        app.active = 1; // on "random"
        app.channels[0].unread_count = 2; // "general" has unread activity
        app.channels[0].first_unread = Some(7);

        app.handle_key(key(KeyCode::Tab)); // wraps to "general"
        assert_eq!(app.active, 0);
        assert_eq!(app.active().unread_count, 0);
        assert_eq!(app.active().first_unread, None);
        assert_eq!(app.active().unread_anchor, Some(7));
    }

    #[test]
    fn received_messages_on_an_inactive_channel_increment_unread_count_from_the_first() {
        let mut app = multi_channel_app();
        app.active = 1; // "random" is active; "general" is not
        let first = compose_message(PEER_ID, "hello", None);
        let first_id = first.id;
        app.handle_net_event(NetEvent::Received("general".to_string(), first));
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "again", None),
        ));

        assert_eq!(app.channels[0].unread_count, 2);
        assert_eq!(app.channels[0].first_unread, Some(first_id));
        assert_eq!(
            app.channels[1].unread_count, 0,
            "active channel is untouched"
        );
    }

    #[test]
    fn received_message_on_active_channel_does_not_mark_unread() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hello", None);
        app.handle_net_event(NetEvent::Received("general".to_string(), message));
        assert_eq!(app.active().unread_count, 0);
        assert_eq!(app.active().first_unread, None);
    }

    #[test]
    fn duplicate_messages_do_not_increment_an_unread_count() {
        let mut app = multi_channel_app();
        app.active = 1; // on "random"
        let message = compose_message(PEER_ID, "hello", None);

        app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
        app.handle_net_event(NetEvent::Received("general".to_string(), message));

        assert_eq!(app.channels[0].unread_count, 1);
    }

    #[test]
    fn scrolling_after_switching_discards_the_unread_anchor() {
        let mut app = multi_channel_app();
        app.active = 1; // on "random"
        app.channels[0].unread_count = 1;
        app.channels[0].first_unread = Some(7);

        app.handle_key(key(KeyCode::Tab));
        app.handle_key(key(KeyCode::Up));

        assert_eq!(app.active().unread_anchor, None);
    }

    #[test]
    fn messages_and_peers_are_isolated_per_channel() {
        let mut app = multi_channel_app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "only in general", None),
        ));

        assert_eq!(app.channels[0].peers, vec![PEER_ID]);
        assert_eq!(app.channels[0].messages.len(), 1);
        assert!(app.channels[1].peers.is_empty());
        assert!(app.channels[1].messages.is_empty());
    }

    #[test]
    fn slash_join_with_arg_returns_join_action() {
        let mut app = app();
        for c in "/join project-x".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let action = app.handle_key(key(KeyCode::Enter));
        match action.expect("non-empty /join returns an action") {
            InputAction::Join(arg) => assert_eq!(arg, "project-x"),
            _ => panic!("expected InputAction::Join"),
        }
        assert_eq!(app.active().input, "");
    }

    #[test]
    fn slash_join_without_arg_shows_usage_and_returns_none() {
        let mut app = app();
        for c in "/join".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"));
    }

    #[test]
    fn slash_join_rejects_the_reserved_device_sync_channel_name() {
        let mut app = app();
        let action = submit(&mut app, "/join _devices");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("reserved"), "got: {last}");
    }

    #[test]
    fn unknown_command_shows_error_and_returns_none() {
        let mut app = app();
        for c in "/bogus".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let action = app.handle_key(key(KeyCode::Enter));
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "unknown command: /bogus");
    }

    #[test]
    fn help_command_pushes_a_system_notice_and_returns_none() {
        let mut app = app();
        let action = submit(&mut app, "/help");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("commands:"), "got: {last}");
    }

    #[test]
    fn help_puts_each_command_on_its_own_line() {
        let mut app = app();
        submit(&mut app, "/help");
        let last = as_system(app.active().messages.back().unwrap());
        for command in ["/join ", "/msg ", "/sync ", "/help "] {
            assert!(
                last.lines().any(|line| line.starts_with(command)),
                "{command} should start its own line, got: {last}"
            );
        }
        assert!(last.contains("/who peer details"), "got: {last}");
        assert!(
            last.lines().any(|line| line == "Ctrl+C -- quit"),
            "got: {last}"
        );
    }

    #[test]
    fn help_lists_the_files_overlay_and_its_keys() {
        let mut app = app();
        submit(&mut app, "/help");
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("/files"), "got: {last}");
        assert!(last.contains("Ctrl+O"), "got: {last}");
    }

    #[test]
    fn hint_hover_reports_a_change_only_when_the_row_changes() {
        let mut app = app();
        assert!(app.set_hint_hover(Some(12)));
        assert_eq!(app.hovered_hint(), Some(12));
        assert!(!app.set_hint_hover(Some(12)));
        assert!(app.set_hint_hover(Some(1)));
        assert_eq!(app.hovered_hint(), Some(1));
        assert!(app.set_hint_hover(None));
        assert_eq!(app.hovered_hint(), None);
        assert!(!app.set_hint_hover(None));
    }

    #[test]
    fn hiding_hints_and_opening_an_overlay_cancel_hover() {
        let mut app = app();
        app.set_hint_hover(Some(12));
        submit(&mut app, "/hints");
        assert!(!app.can_hover_hints());
        assert!(app.hint_hover.is_none());
        assert!(!app.set_hint_hover(Some(12)));
        submit(&mut app, "/hints");
        app.set_hint_hover(Some(12));
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(!app.can_hover_hints());
        assert_eq!(app.hovered_hint(), None);
        assert!(app.set_hint_hover(None));
    }

    #[test]
    fn opening_the_command_picker_cancels_hint_hover() {
        let mut app = app();
        app.set_hint_hover(Some(12));
        app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
        assert!(app.command_picker.is_some());
        assert!(!app.can_hover_hints());
        assert_eq!(app.hovered_hint(), None);
    }

    #[test]
    fn hints_are_shown_by_default() {
        assert!(app().show_hints);
    }

    #[test]
    fn slash_hints_toggles_visibility_and_reports_state() {
        let mut app = app();

        submit(&mut app, "/hints");
        assert!(!app.show_hints);
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "command hints hidden");

        submit(&mut app, "/hints");
        assert!(app.show_hints);
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "command hints shown");
    }

    #[test]
    fn bell_is_enabled_by_default() {
        assert!(app().bell_enabled);
    }

    #[test]
    fn slash_bell_toggles_and_reports_state_and_returns_an_action() {
        let mut app = app();

        let action = submit(&mut app, "/bell");
        assert!(!app.bell_enabled);
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "message bell off");
        match action.expect("/bell returns an action") {
            InputAction::Bell(enabled) => assert!(!enabled),
            _ => panic!("expected InputAction::Bell"),
        }

        let action = submit(&mut app, "/bell");
        assert!(app.bell_enabled);
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "message bell on");
        match action.expect("/bell returns an action") {
            InputAction::Bell(enabled) => assert!(enabled),
            _ => panic!("expected InputAction::Bell"),
        }
    }

    #[test]
    fn load_settings_seeds_bell_enabled_and_local_nickname() {
        let mut app = app();
        app.load_settings(false, Some("me".to_string()));
        assert!(!app.bell_enabled);
        assert_eq!(app.local_nickname.as_deref(), Some("me"));
    }

    #[test]
    fn slash_nick_updates_this_devices_who_row() {
        let mut app = app();

        let action = submit(&mut app, "/nick me");
        submit(&mut app, "/who");

        assert!(matches!(action, Some(InputAction::Nick(name)) if name == "me"));
        let self_row = app
            .viewing_peers
            .as_ref()
            .unwrap()
            .peers
            .iter()
            .find(|peer| peer.id == SELF_ID)
            .unwrap();
        assert_eq!(self_row.nickname.as_deref(), Some("me"));
    }

    #[test]
    fn slash_invite_returns_invite_action_for_active_channel() {
        let mut app = multi_channel_app();
        for c in "/invite".chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
        let action = app.handle_key(key(KeyCode::Enter));
        match action.expect("/invite returns an action") {
            InputAction::Invite(channel) => assert_eq!(channel, "random"),
            _ => panic!("expected InputAction::Invite"),
        }
    }

    #[test]
    fn slash_paste_returns_paste_action_for_active_channel() {
        let mut app = multi_channel_app();
        let action = submit(&mut app, "/paste");
        match action.expect("/paste returns an action") {
            InputAction::Paste(channel) => assert_eq!(channel, "random"),
            _ => panic!("expected InputAction::Paste"),
        }
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("clipboard"), "got: {last}");
    }

    #[test]
    fn ctrl_v_triggers_the_same_paste_action_as_the_slash_command() {
        let mut app = app();
        let action = app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL));
        match action.expect("Ctrl+V returns an action") {
            InputAction::Paste(channel) => assert_eq!(channel, "general"),
            _ => panic!("expected InputAction::Paste"),
        }
    }

    #[test]
    fn super_v_also_triggers_paste_best_effort_for_macos() {
        let mut app = app();
        let action = app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::SUPER));
        assert!(matches!(action, Some(InputAction::Paste(_))));
    }

    #[test]
    fn plain_v_without_a_modifier_just_types_the_character() {
        let mut app = app();
        let action = app.handle_key(key(KeyCode::Char('v')));
        assert!(action.is_none());
        assert_eq!(app.active().input, "v");
    }

    #[test]
    fn compose_own_message_matches_the_calling_apps_identity() {
        let app = app();
        let message = app.compose_own_message("hello from clipboard");
        assert_eq!(message.sender, SELF_ID);
        assert_eq!(message.text, "hello from clipboard");
        assert!(message.attachment.is_none());
    }

    /// A staged attachment named `filename`, as `files::stage_image` would
    /// build it -- the tests below only care that attachments are kept,
    /// ordered, and sent, not where their bytes come from.
    fn staged(filename: &str) -> StagedAttachment {
        StagedAttachment {
            filename: filename.to_string(),
            size: 10,
            source: crate::files::StagedSource::Bytes(vec![0; 10]),
        }
    }

    #[test]
    fn staging_an_attachment_puts_it_on_the_named_channel_only() {
        let mut app = multi_channel_app(); // active: "random"
        app.stage_attachment("general", staged("map.png"));

        assert_eq!(app.channels[0].name, "general");
        assert_eq!(app.channels[0].staged, vec![staged("map.png")]);
        assert!(app.channels[1].staged.is_empty());
    }

    #[test]
    fn staging_for_a_channel_that_is_gone_is_a_no_op() {
        let mut app = app();
        app.stage_attachment("left-meanwhile", staged("map.png"));
        assert!(app.active().staged.is_empty());
    }

    #[test]
    fn enter_sends_every_staged_attachment_and_clears_the_stage() {
        let mut app = app();
        app.stage_attachment("general", staged("a.png"));
        app.stage_attachment("general", staged("b.pdf"));

        let action = app.handle_key(key(KeyCode::Enter));

        match action.expect("Enter with staged attachments returns an action") {
            InputAction::SendStaged {
                channel,
                items,
                caption,
            } => {
                assert_eq!(channel, "general");
                assert_eq!(items, vec![staged("a.png"), staged("b.pdf")]);
                assert_eq!(caption, "", "nothing was typed, so no caption");
            }
            _ => panic!("expected InputAction::SendStaged"),
        }
        assert!(app.active().staged.is_empty());
    }

    #[test]
    fn typed_text_becomes_the_caption_of_the_staged_attachments() {
        let mut app = app();
        app.stage_attachment("general", staged("map.png"));

        let action = submit(&mut app, "the new tilemap");

        match action.expect("returns an action") {
            InputAction::SendStaged { items, caption, .. } => {
                assert_eq!(caption, "the new tilemap");
                assert_eq!(items.len(), 1);
            }
            _ => panic!("expected InputAction::SendStaged"),
        }
        assert_eq!(
            app.active().input,
            "",
            "the caption is consumed from the input box"
        );
    }

    #[test]
    fn sending_staged_attachments_leaves_a_breadcrumb_per_file_and_no_chat_message() {
        let mut app = app();
        app.stage_attachment("general", staged("a.png"));
        app.stage_attachment("general", staged("b.pdf"));
        let before = app.active().messages.len();

        submit(&mut app, "hello");

        // Only system breadcrumbs: the attachments' chat messages arrive
        // later from the importer (`NetEvent::FileReady`), with the caption
        // riding on the first of them.
        let added: Vec<&TranscriptLine> = app.active().messages.iter().skip(before).collect();
        assert_eq!(added.len(), 2);
        assert_eq!(as_system(added[0]), "sending a.png...");
        assert_eq!(as_system(added[1]), "sending b.pdf...");
    }

    #[test]
    fn whitespace_only_input_still_sends_staged_attachments_without_a_caption() {
        let mut app = app();
        app.stage_attachment("general", staged("map.png"));
        app.paste_text("   ");

        let action = app.handle_key(key(KeyCode::Enter));

        match action.expect("returns an action") {
            InputAction::SendStaged { caption, .. } => assert_eq!(caption, ""),
            _ => panic!("expected InputAction::SendStaged"),
        }
        assert_eq!(app.active().input, "");
    }

    #[test]
    fn a_slash_command_runs_as_a_command_and_leaves_the_stage_alone() {
        let mut app = app();
        app.stage_attachment("general", staged("map.png"));

        let action = submit(&mut app, "/who");

        assert!(action.is_none(), "/who is a local command, not a send");
        assert_eq!(app.active().staged, vec![staged("map.png")]);
    }

    #[test]
    fn slash_send_still_sends_immediately_even_with_attachments_staged() {
        let mut app = app();
        app.stage_attachment("general", staged("staged.png"));

        let action = submit(&mut app, "/send ~/other.png");

        match action.expect("/send returns an action") {
            InputAction::SendFile { channel, path } => {
                assert_eq!(channel, "general");
                assert_eq!(path, "~/other.png");
            }
            _ => panic!("expected InputAction::SendFile"),
        }
        assert_eq!(app.active().staged.len(), 1, "the stage is untouched");
    }

    #[test]
    fn backspace_on_an_empty_input_unstages_the_newest_attachment() {
        let mut app = app();
        app.stage_attachment("general", staged("first.png"));
        app.stage_attachment("general", staged("second.png"));

        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.active().staged, vec![staged("first.png")]);

        app.handle_key(key(KeyCode::Backspace));
        assert!(app.active().staged.is_empty());
    }

    #[test]
    fn backspace_with_text_typed_edits_the_text_and_keeps_the_stage() {
        let mut app = app();
        app.stage_attachment("general", staged("map.png"));
        app.handle_key(key(KeyCode::Char('h')));
        app.handle_key(key(KeyCode::Char('i')));

        app.handle_key(key(KeyCode::Backspace));

        assert_eq!(app.active().input, "h");
        assert_eq!(app.active().staged.len(), 1);
    }

    #[test]
    fn esc_clears_the_stage_before_it_quits() {
        let mut app = app();
        app.stage_attachment("general", staged("a.png"));
        app.stage_attachment("general", staged("b.png"));

        app.handle_key(key(KeyCode::Esc));
        assert!(app.active().staged.is_empty());
        assert!(!app.should_quit, "the first Esc only clears the stage");

        app.handle_key(key(KeyCode::Esc));
        assert!(app.should_quit);
    }

    #[test]
    fn esc_cancels_an_armed_reply_before_clearing_the_stage() {
        let mut app = app();
        app.stage_attachment("general", staged("a.png"));
        app.replying_to = Some(1);

        app.handle_key(key(KeyCode::Esc));

        assert!(app.replying_to.is_none());
        assert_eq!(
            app.active().staged.len(),
            1,
            "the reply is backed out of first; the stage survives"
        );
    }

    #[test]
    fn staged_attachments_stay_with_their_channel_across_tab_switches() {
        let mut app = multi_channel_app(); // starts on "random"
        app.stage_attachment("random", staged("map.png"));

        app.handle_key(key(KeyCode::Tab)); // wraps around to "general"
        assert_eq!(app.active().name, "general");
        assert!(
            app.active().staged.is_empty(),
            "another channel's stage must not show up here"
        );

        // Enter in "general" sends plain text, never random's attachment.
        let action = submit(&mut app, "hi");
        assert!(matches!(action, Some(InputAction::Send(..))));

        app.handle_key(key(KeyCode::Tab)); // back to "random"
        assert_eq!(app.active().staged, vec![staged("map.png")]);
    }

    #[test]
    fn a_fresh_session_is_composing() {
        assert!(app().is_composing());
    }

    #[test]
    fn composing_stops_while_the_switcher_is_open() {
        let mut app = app();
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
        assert!(!app.is_composing());

        app.handle_key(key(KeyCode::Esc));
        assert!(app.is_composing());
    }

    #[test]
    fn composing_stops_during_a_reply_or_thread_pick_and_a_pairing_confirmation() {
        let mut picking_reply = app();
        picking_reply.picking_reply = Some(1);
        assert!(!picking_reply.is_composing());

        let mut picking_thread = app();
        picking_thread.picking_thread = Some(1);
        assert!(!picking_thread.is_composing());

        let mut redeeming = app();
        redeeming.pending_redeem = Some("ticket".to_string());
        assert!(!redeeming.is_composing());
    }

    // -- the file picker, opened by a bare /send -----------------------------

    use crate::picker::{Entry, Kind};

    fn picked_file(name: &str) -> Entry {
        Entry::new(
            name.to_string(),
            Kind::File {
                size: 1,
                modified: std::time::SystemTime::UNIX_EPOCH,
            },
        )
    }

    fn picked_folder(name: &str) -> Entry {
        Entry::new(name.to_string(), Kind::Folder)
    }

    fn listing(request: u64, entries: Vec<Entry>) -> FolderListing {
        FolderListing {
            request,
            result: Ok(entries),
        }
    }

    /// The request id of the folder read `action` asks for.
    fn read_request(action: Option<InputAction>) -> u64 {
        match action {
            Some(InputAction::ReadFolder { request, .. }) => request,
            _ => panic!("expected InputAction::ReadFolder"),
        }
    }

    /// Opens the picker with a bare `/send` on `app`, whose picker folder is
    /// `/pics`, and answers its read with a folder and two files.
    fn open_picker(app: &mut AppState) {
        app.load_browse_dir(PathBuf::from("/pics"));
        let request = read_request(submit(app, "/send"));
        app.apply_folder_listing(listing(
            request,
            vec![
                picked_folder("docs"),
                picked_file("a.txt"),
                picked_file("b.txt"),
            ],
        ));
    }

    #[test]
    fn a_bare_send_opens_the_picker_and_asks_for_the_remembered_folder_to_be_read() {
        let mut app = app();
        app.load_browse_dir(PathBuf::from("/pics"));

        let action = submit(&mut app, "/send");

        match action {
            Some(InputAction::ReadFolder { request, dir }) => {
                assert_eq!(request, 1);
                assert_eq!(dir, PathBuf::from("/pics"));
            }
            _ => panic!("expected InputAction::ReadFolder"),
        }
        let picker = app.picking_file.as_ref().expect("the picker is open");
        assert_eq!(picker.dir(), Path::new("/pics"));
        assert!(picker.is_reading());
        assert_eq!(app.active().input, "", "the command was consumed");
        assert!(app.active().messages.is_empty(), "no usage notice any more");
    }

    #[test]
    fn the_picker_opens_at_the_root_until_a_folder_is_seeded() {
        let mut app = app();

        match submit(&mut app, "/send") {
            Some(InputAction::ReadFolder { dir, .. }) => {
                assert_eq!(dir, PathBuf::from(std::path::MAIN_SEPARATOR_STR));
            }
            _ => panic!("expected InputAction::ReadFolder"),
        }
    }

    #[test]
    fn send_with_a_path_still_sends_it_and_leaves_the_picker_closed() {
        let mut app = app();

        let action = submit(&mut app, "/send ~/Downloads/a.png");

        assert!(matches!(action, Some(InputAction::SendFile { .. })));
        assert!(app.picking_file.is_none());
    }

    #[test]
    fn the_picker_owns_keys_and_pastes_while_open() {
        let mut app = app();
        open_picker(&mut app);
        assert!(!app.is_composing());

        app.handle_key(key(KeyCode::Char('a')));
        app.paste_text("txt\n");

        assert_eq!(app.active().input, "", "nothing lands in the draft");
        let picker = app.picking_file.as_ref().expect("still open");
        assert_eq!(
            picker.query(),
            "atxt",
            "a paste filters, minus its line break"
        );
    }

    #[test]
    fn the_picker_answers_keys_before_its_folder_has_arrived() {
        let mut app = app();
        app.load_browse_dir(PathBuf::from("/pics"));
        submit(&mut app, "/send"); // never answered: a huge or slow folder

        assert!(app.handle_key(key(KeyCode::Down)).is_none());
        assert!(app.handle_key(key(KeyCode::Char('x'))).is_none());
        assert!(app.handle_key(key(KeyCode::Enter)).is_none());
        assert!(app.picking_file.is_some(), "still open, and still waiting");

        app.handle_key(key(KeyCode::Esc));
        assert!(app.picking_file.is_none(), "and Esc still closes it");
    }

    #[test]
    fn escape_closes_the_picker_without_staging_or_quitting() {
        let mut app = app();
        open_picker(&mut app);

        let action = app.handle_key(key(KeyCode::Esc));

        assert!(action.is_none());
        assert!(app.picking_file.is_none());
        assert!(!app.should_quit);
        assert!(app.is_composing());
    }

    #[test]
    fn ctrl_c_still_quits_from_the_picker() {
        let mut app = app();
        open_picker(&mut app);

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(app.should_quit);
    }

    #[test]
    fn choosing_a_file_stages_it_on_the_channel_the_picker_was_opened_on() {
        let mut app = multi_channel_app(); // "random" is active
        open_picker(&mut app);
        app.handle_key(key(KeyCode::Down)); // a.txt
        app.active = 0; // a join switched channels meanwhile

        let action = app.handle_key(key(KeyCode::Enter));

        match action {
            Some(InputAction::StageFiles {
                channel,
                paths,
                folder,
            }) => {
                assert_eq!(channel, "random");
                assert_eq!(paths, [PathBuf::from("/pics/a.txt")]);
                assert_eq!(folder, PathBuf::from("/pics"));
            }
            _ => panic!("expected InputAction::StageFiles"),
        }
        assert!(app.picking_file.is_none(), "choosing closes the picker");
    }

    #[test]
    fn marked_files_are_all_staged_together() {
        let mut app = app();
        open_picker(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Char(' '))); // a.txt
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Char(' '))); // b.txt

        match app.handle_key(key(KeyCode::Enter)) {
            Some(InputAction::StageFiles { paths, .. }) => assert_eq!(
                paths,
                [PathBuf::from("/pics/a.txt"), PathBuf::from("/pics/b.txt")]
            ),
            _ => panic!("expected InputAction::StageFiles"),
        }
    }

    #[test]
    fn the_folder_files_were_chosen_from_is_where_the_picker_opens_next() {
        let mut app = app();
        open_picker(&mut app);
        // Into docs, then choose a file there.
        let request = read_request(app.handle_key(key(KeyCode::Enter)));
        app.apply_folder_listing(listing(request, vec![picked_file("deep.txt")]));
        app.handle_key(key(KeyCode::Enter));

        match submit(&mut app, "/send") {
            Some(InputAction::ReadFolder { dir, .. }) => {
                assert_eq!(dir, PathBuf::from("/pics/docs"));
            }
            _ => panic!("expected InputAction::ReadFolder"),
        }
    }

    #[test]
    fn cancelling_does_not_change_where_the_picker_opens_next() {
        let mut app = app();
        open_picker(&mut app);
        let request = read_request(app.handle_key(key(KeyCode::Enter))); // into docs
        app.apply_folder_listing(listing(request, vec![picked_file("deep.txt")]));
        app.handle_key(key(KeyCode::Esc));

        match submit(&mut app, "/send") {
            Some(InputAction::ReadFolder { dir, .. }) => {
                assert_eq!(dir, PathBuf::from("/pics"));
            }
            _ => panic!("expected InputAction::ReadFolder"),
        }
    }

    #[test]
    fn opening_a_folder_asks_for_it_under_a_fresh_request_id() {
        let mut app = app();
        open_picker(&mut app); // request 1

        match app.handle_key(key(KeyCode::Enter)) {
            Some(InputAction::ReadFolder { request, dir }) => {
                assert_eq!(request, 2);
                assert_eq!(dir, PathBuf::from("/pics/docs"));
            }
            _ => panic!("expected InputAction::ReadFolder"),
        }
    }

    #[test]
    fn going_up_asks_for_the_parent_folder() {
        let mut app = app();
        open_picker(&mut app);
        let request = read_request(app.handle_key(key(KeyCode::Enter))); // into docs
        app.apply_folder_listing(listing(request, vec![picked_file("deep.txt")]));

        match app.handle_key(key(KeyCode::Backspace)) {
            Some(InputAction::ReadFolder { dir, .. }) => {
                assert_eq!(dir, PathBuf::from("/pics"));
            }
            _ => panic!("expected InputAction::ReadFolder"),
        }
    }

    #[test]
    fn an_answer_only_reaches_the_picker_waiting_for_it() {
        let mut app = app();
        app.load_browse_dir(PathBuf::from("/pics"));
        let first = read_request(submit(&mut app, "/send"));
        app.handle_key(key(KeyCode::Esc));
        let second = read_request(submit(&mut app, "/send"));
        assert_ne!(first, second, "every read has an id of its own");

        // The first picker's slow answer turns up after the second opened.
        app.apply_folder_listing(listing(first, vec![picked_file("stale.txt")]));
        let picker = app.picking_file.as_ref().expect("open");
        assert!(picker.is_reading(), "still waiting for its own");
        assert_eq!(picker.visible().len(), 0);

        app.apply_folder_listing(listing(second, vec![picked_file("fresh.txt")]));
        let picker = app.picking_file.as_ref().expect("open");
        assert!(!picker.is_reading());
        assert_eq!(picker.visible().len(), 1);
    }

    #[test]
    fn an_answer_after_the_picker_closed_is_ignored() {
        let mut app = app();
        app.load_browse_dir(PathBuf::from("/pics"));
        let request = read_request(submit(&mut app, "/send"));
        app.handle_key(key(KeyCode::Esc));

        app.apply_folder_listing(listing(request, vec![picked_file("late.txt")]));

        assert!(app.picking_file.is_none());
    }

    #[test]
    fn slash_leave_with_no_arg_leaves_the_active_channel() {
        let mut app = multi_channel_app();
        let action = submit(&mut app, "/leave");
        match action.expect("/leave returns an action") {
            InputAction::Leave(channel) => assert_eq!(channel, "random"),
            _ => panic!("expected InputAction::Leave"),
        }
    }

    #[test]
    fn slash_leave_with_a_name_leaves_that_channel_even_if_inactive() {
        let mut app = multi_channel_app();
        assert_eq!(app.active().name, "random");
        let action = submit(&mut app, "/leave general");
        match action.expect("/leave returns an action") {
            InputAction::Leave(channel) => assert_eq!(channel, "general"),
            _ => panic!("expected InputAction::Leave"),
        }
    }

    #[test]
    fn slash_leave_refuses_an_unjoined_channel() {
        let mut app = multi_channel_app();
        let action = submit(&mut app, "/leave nonexistent");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "not currently in #nonexistent");
    }

    #[test]
    fn slash_leave_refuses_the_only_channel() {
        let mut app = app();
        let action = submit(&mut app, "/leave");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "cannot leave your only channel");
    }

    #[test]
    fn slash_who_includes_this_devices_endpoint_without_a_transcript_notice() {
        let mut app = app();
        let messages = app.active().messages.len();

        let action = submit(&mut app, "/who");

        assert!(action.is_none());
        let overlay = app.viewing_peers.as_ref().expect("/who opens an overlay");
        assert_eq!(overlay.heading, "#general");
        assert_eq!(
            overlay
                .peers
                .iter()
                .map(|peer| (peer.id, peer.alias.as_deref(), peer.nickname.as_deref()))
                .collect::<Vec<_>>(),
            vec![(SELF_ID, None, None)]
        );
        assert_eq!(app.active().messages.len(), messages);
        assert!(!app.is_composing());
    }

    #[test]
    fn slash_who_snapshots_endpoint_alias_and_nickname_for_each_active_peer() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID_2));
        app.load_contacts(HashMap::from([(PEER_ID, "Bob".to_string())]));
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "alice".to_string(),
            },
        ));

        submit(&mut app, "/who");

        let overlay = app.viewing_peers.as_ref().unwrap();
        assert_eq!(
            overlay
                .peers
                .iter()
                .map(|peer| (peer.id, peer.alias.as_deref(), peer.nickname.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                (PEER_ID, Some("Bob"), Some("alice")),
                (PEER_ID_2, None, None),
                (SELF_ID, None, None),
            ]
        );
    }

    #[test]
    fn slash_who_only_snapshots_the_active_channels_peers() {
        let mut app = multi_channel_app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert_eq!(app.active().name, "random");

        submit(&mut app, "/who");

        let overlay = app.viewing_peers.as_ref().unwrap();
        assert_eq!(overlay.heading, "#random");
        assert_eq!(
            overlay.peers.iter().map(|peer| peer.id).collect::<Vec<_>>(),
            [SELF_ID]
        );
    }

    #[test]
    fn slash_who_lists_online_peers_before_remembered_offline_peers() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.record_peer_seen("general", PEER_ID_2, 100);

        submit(&mut app, "/who");

        assert_eq!(
            app.viewing_peers
                .as_ref()
                .unwrap()
                .peers
                .iter()
                .map(|peer| (peer.id, peer.last_seen_unix_ms))
                .collect::<Vec<_>>(),
            vec![(PEER_ID, None), (SELF_ID, None), (PEER_ID_2, Some(100))]
        );
    }

    #[test]
    fn slash_who_deduplicates_certified_devices_like_the_peer_sidebar() {
        let mut app = app();
        let user_key = SecretKey::generate();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID_2));
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID_2),
        ));
        app.load_contacts(HashMap::from([(
            *user_key.public().as_bytes(),
            "Alice".to_string(),
        )]));

        submit(&mut app, "/who");

        let overlay = app.viewing_peers.as_ref().unwrap();
        assert_eq!(
            overlay.peers.iter().map(|peer| peer.id).collect::<Vec<_>>(),
            vec![PEER_ID, SELF_ID]
        );
        assert!(
            overlay
                .peers
                .iter()
                .filter(|peer| peer.id != SELF_ID)
                .all(|peer| peer.alias.as_deref() == Some("Alice"))
        );
    }

    #[test]
    fn esc_closes_the_who_overlay_and_its_paste_never_reaches_the_draft() {
        let mut app = app();
        submit(&mut app, "/who");
        app.paste_text("hidden");

        app.handle_key(key(KeyCode::Esc));

        assert!(app.viewing_peers.is_none());
        assert!(app.is_composing());
        assert!(!app.should_quit);
        assert!(app.active().input.is_empty());
    }

    #[test]
    fn remove_channel_switches_active_to_the_previous_tab_when_removing_the_active_one() {
        let mut app = multi_channel_app();
        assert_eq!(app.active().name, "random");
        app.remove_channel("random");
        assert_eq!(app.channels.len(), 1);
        assert_eq!(app.active().name, "general");
    }

    #[test]
    fn remove_channel_keeps_the_active_tab_selected_when_removing_a_different_one() {
        let mut app = multi_channel_app();
        assert_eq!(app.active().name, "random");
        app.remove_channel("general");
        assert_eq!(app.channels.len(), 1);
        assert_eq!(
            app.active().name,
            "random",
            "removing an earlier tab must not change which channel is active"
        );
    }

    #[test]
    fn remove_channel_pushes_a_left_system_notice() {
        let mut app = multi_channel_app();
        app.remove_channel("random");
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "left #random");
    }

    #[test]
    fn remove_channel_is_a_no_op_for_an_unknown_channel() {
        let mut app = multi_channel_app();
        app.remove_channel("nonexistent");
        assert_eq!(app.channels.len(), 2);
        assert_eq!(app.active().name, "random");
    }

    #[test]
    fn joined_event_adds_new_channel_and_switches_to_it() {
        let mut app = app();
        app.handle_net_event(NetEvent::Joined("project-x".to_string()));
        assert_eq!(app.channels.len(), 2);
        assert_eq!(app.active().name, "project-x");
    }

    #[test]
    fn joined_event_on_existing_channel_switches_without_duplicating() {
        let mut app = multi_channel_app();
        app.active = 0;
        app.handle_net_event(NetEvent::Joined("random".to_string()));
        assert_eq!(
            app.channels.len(),
            2,
            "must not duplicate an existing channel"
        );
        assert_eq!(app.active().name, "random");
    }

    #[test]
    fn channel_sync_joined_adds_a_background_tab_without_switching_active() {
        // Unlike a plain `Joined` (the user explicitly ran /join), a
        // channel learned via the private device-sync channel must not
        // yank focus away from whatever the user is currently looking at.
        let mut app = app();
        assert_eq!(app.active().name, "general");

        app.handle_net_event(NetEvent::ChannelSyncJoined("project-x".to_string()));

        assert_eq!(app.channels.len(), 2);
        assert_eq!(
            app.active().name,
            "general",
            "a background sync join must not switch the active tab"
        );
        assert!(app.channels.iter().any(|c| c.name == "project-x"));
    }

    #[test]
    fn channel_sync_joined_on_an_already_known_channel_does_not_duplicate() {
        let mut app = multi_channel_app();
        app.handle_net_event(NetEvent::ChannelSyncJoined("random".to_string()));
        assert_eq!(app.channels.len(), 2, "must not duplicate an existing tab");
    }

    #[test]
    fn lagged_pushes_system_message_on_the_affected_channel() {
        let mut app = multi_channel_app();
        app.active = 1; // "random" is active; "general" is not

        app.handle_net_event(NetEvent::Lagged("general".to_string()));

        let last = as_system(app.channels[0].messages.back().unwrap());
        assert!(last.contains("lagged"));
        assert!(
            app.channels[1].messages.is_empty(),
            "lag on another channel shouldn't touch the active one"
        );
    }

    #[test]
    fn join_failed_pushes_system_message_on_active_channel() {
        let mut app = app();
        app.handle_net_event(NetEvent::JoinFailed(
            "project-x".to_string(),
            "boom".to_string(),
        ));
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "failed to join #project-x: boom");
    }

    #[test]
    fn handle_net_event_returns_channel_and_message_for_a_new_received_message() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hello", None);
        let result =
            app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
        match result {
            Some((channel, returned)) => {
                assert_eq!(channel, "general");
                assert_eq!(returned, message);
            }
            None => panic!("expected Some for a newly-received message"),
        }
    }

    #[test]
    fn handle_net_event_returns_none_for_a_duplicate_received_message() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hello", None);
        app.handle_net_event(NetEvent::Received("general".to_string(), message.clone()));
        let result = app.handle_net_event(NetEvent::Received("general".to_string(), message));
        assert!(result.is_none());
    }

    #[test]
    fn handle_net_event_returns_none_for_non_received_events() {
        let mut app = app();
        let result = app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        assert!(result.is_none());
    }

    #[test]
    fn load_history_seeds_transcript_and_dedupes_like_live_messages() {
        let mut app = app();
        let message = compose_message(PEER_ID, "from before restart", None);
        app.load_history("general", vec![message.clone()]);

        assert_eq!(app.active().messages.len(), 1);
        assert_eq!(as_chat(app.active().messages.back().unwrap()), &message);

        // A live redelivery of the same id must still be deduped.
        let before = app.active().messages.len();
        app.handle_net_event(NetEvent::Received("general".to_string(), message));
        assert_eq!(app.active().messages.len(), before);
    }

    #[test]
    fn load_history_on_an_unjoined_channel_is_a_no_op() {
        let mut app = app();
        app.load_history("nonexistent", vec![compose_message(PEER_ID, "hi", None)]);
        assert_eq!(app.active().messages.len(), 0);
    }

    #[test]
    fn an_old_file_share_stays_listed_and_saveable_after_scrollback_trims_it() {
        let mut app = app();
        let old = file_message(1, PEER_ID, "old.pdf", b"old");
        let hash = old.attachment.as_ref().unwrap().hash;
        let mut history = vec![old];
        for id in 2..=MAX_SCROLLBACK as u64 + 1 {
            history.push(compose_message(SELF_ID, &format!("message {id}"), None));
        }
        app.load_history("general", history);

        assert_eq!(app.active().messages.len(), MAX_SCROLLBACK);
        open_files(&mut app);
        assert_eq!(listed(&app), vec!["old.pdf"]);
        app.handle_key(key(KeyCode::Esc));

        let (channel, got, filename, providers, _) =
            save_fields(submit(&mut app, &format!("/save {}", &hash.to_hex()[..8])));
        assert_eq!(
            (channel, got, filename),
            ("general".into(), hash, "old.pdf".into())
        );
        assert_eq!(providers, vec![PEER_ID]);
    }

    fn dated_message(ts_unix_ms: u64, id: u64, sender: [u8; 32], text: &str) -> ChatMessage {
        ChatMessage {
            v: 1,
            id,
            sender,
            ts_unix_ms,
            text: text.to_string(),
            attachment: None,
            reply_to: None,
        }
    }

    fn reply_message(
        ts_unix_ms: u64,
        id: u64,
        sender: [u8; 32],
        text: &str,
        reply_to: u64,
    ) -> ChatMessage {
        ChatMessage {
            reply_to: Some(reply_to),
            ..dated_message(ts_unix_ms, id, sender, text)
        }
    }

    fn file_message(id: u64, sender: [u8; 32], filename: &str, content: &[u8]) -> ChatMessage {
        ChatMessage {
            v: 2,
            id,
            sender,
            ts_unix_ms: id,
            text: String::new(),
            attachment: Some(crate::message::FileAttachment {
                filename: filename.to_string(),
                size: content.len() as u64,
                hash: iroh_blobs::Hash::new(content),
            }),
            reply_to: None,
        }
    }

    #[test]
    fn merge_history_dedupes_against_already_known_messages() {
        let mut app = app();
        let known = dated_message(100, 1, PEER_ID, "known");
        app.load_history("general", vec![known.clone()]);

        let new_message = dated_message(200, 2, PEER_ID, "new");
        let accepted = app.merge_history("general", vec![known, new_message.clone()]);

        assert_eq!(accepted, vec![new_message]);
        assert_eq!(app.active().messages.len(), 2);
    }

    #[test]
    fn merge_history_sorts_backfilled_messages_by_timestamp_not_append_order() {
        let mut app = app();
        let recent = dated_message(200, 1, PEER_ID, "recent");
        app.load_history("general", vec![recent.clone()]);

        let older = dated_message(100, 2, PEER_ID, "older");
        app.merge_history("general", vec![older.clone()]);

        let messages: Vec<&ChatMessage> = app.active().messages.iter().map(as_chat).collect();
        assert_eq!(
            messages,
            vec![&older, &recent],
            "an older backfilled message must sort before what was already there"
        );
    }

    #[test]
    fn merge_history_returns_nothing_when_everything_is_already_known() {
        let mut app = app();
        let message = dated_message(100, 1, PEER_ID, "hi");
        app.load_history("general", vec![message.clone()]);

        let accepted = app.merge_history("general", vec![message]);
        assert!(accepted.is_empty());
    }

    #[test]
    fn merge_history_on_an_unjoined_channel_is_a_no_op() {
        let mut app = app();
        let accepted = app.merge_history("nonexistent", vec![dated_message(100, 1, PEER_ID, "hi")]);
        assert!(accepted.is_empty());
        assert_eq!(app.active().messages.len(), 0);
    }

    #[test]
    fn merge_history_keeps_system_lines_after_the_resorted_chat_messages() {
        let mut app = app();
        app.push_system("earlier notice");
        let older = dated_message(100, 1, PEER_ID, "older");
        app.merge_history("general", vec![older.clone()]);

        let lines: Vec<&TranscriptLine> = app.active().messages.iter().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(as_chat(lines[0]), &older);
        assert_eq!(as_system(lines[1]), "earlier notice");
    }

    #[test]
    fn merge_history_enforces_the_scrollback_cap_evicting_the_oldest_first() {
        let mut app = app();
        let batch: Vec<ChatMessage> = (0..(MAX_SCROLLBACK as u64 + 10))
            .map(|i| dated_message(i, i, PEER_ID, "msg"))
            .collect();

        let accepted = app.merge_history("general", batch);

        assert_eq!(accepted.len(), MAX_SCROLLBACK + 10);
        assert_eq!(app.active().messages.len(), MAX_SCROLLBACK);
        let oldest_remaining = as_chat(app.active().messages.front().unwrap());
        assert_eq!(
            oldest_remaining.id, 10,
            "the 10 oldest messages must have been evicted to stay within the cap"
        );
    }

    #[test]
    fn slash_search_without_arg_and_no_active_filter_shows_usage() {
        let mut app = app();
        let action = submit(&mut app, "/search");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"), "got: {last}");
    }

    #[test]
    fn slash_search_finds_matches_in_loaded_history_and_returns_an_action() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hello world", None),
        ));
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "goodbye", None),
        ));

        let action = submit(&mut app, "/search world");

        match action.expect("a successful /search returns an action") {
            InputAction::Search { channel, term } => {
                assert_eq!(channel, "general");
                assert_eq!(term, "world");
            }
            _ => panic!("expected InputAction::Search"),
        }
        let search = app
            .active()
            .search
            .as_ref()
            .expect("search should be active");
        assert_eq!(search.messages.len(), 1);
        assert_eq!(search.messages[0].text, "hello world");
        assert!(search.pending, "the on-disk scan hasn't reported back yet");
    }

    #[test]
    fn slash_search_is_case_insensitive() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "Hello World", None),
        ));

        submit(&mut app, "/search WORLD");

        let search = app
            .active()
            .search
            .as_ref()
            .expect("search should be active");
        assert_eq!(search.messages.len(), 1);
    }

    #[test]
    fn slash_search_with_no_matches_still_activates_an_empty_search() {
        let mut app = app();
        let action = submit(&mut app, "/search nonexistent");
        assert!(action.is_some());
        let search = app
            .active()
            .search
            .as_ref()
            .expect("search should be active");
        assert!(search.messages.is_empty());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("0 match"), "got: {last}");
    }

    #[test]
    fn slash_search_with_no_arg_clears_an_active_filter() {
        let mut app = app();
        submit(&mut app, "/search hello");
        assert!(app.active().search.is_some());

        let action = submit(&mut app, "/search");

        assert!(action.is_none());
        assert!(app.active().search.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "search cleared");
    }

    #[test]
    fn apply_search_outcome_merges_on_disk_matches_into_existing_results() {
        let mut app = app();
        submit(&mut app, "/search hello");
        let older = dated_message(50, 99, PEER_ID, "hello from the past");

        app.apply_search_outcome(SearchOutcome {
            channel: "general".to_string(),
            term: "hello".to_string(),
            messages: vec![older.clone()],
        });

        let search = app.active().search.as_ref().unwrap();
        assert_eq!(search.messages, vec![older]);
        assert!(!search.pending);
    }

    #[test]
    fn apply_search_outcome_deduplicates_messages_already_in_the_snapshot() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hello there", None),
        ));
        let action = submit(&mut app, "/search hello");
        let term = match action.expect("search returns an action") {
            InputAction::Search { term, .. } => term,
            _ => panic!("expected InputAction::Search"),
        };
        let already_shown = app.active().search.as_ref().unwrap().messages[0].clone();

        app.apply_search_outcome(SearchOutcome {
            channel: "general".to_string(),
            term,
            messages: vec![already_shown.clone()],
        });

        let search = app.active().search.as_ref().unwrap();
        assert_eq!(search.messages, vec![already_shown]);
    }

    #[test]
    fn apply_search_outcome_ignores_a_stale_result_from_a_superseded_search() {
        let mut app = app();
        submit(&mut app, "/search first");
        submit(&mut app, "/search second");

        app.apply_search_outcome(SearchOutcome {
            channel: "general".to_string(),
            term: "first".to_string(),
            messages: vec![dated_message(1, 1, PEER_ID, "first match")],
        });

        let search = app.active().search.as_ref().unwrap();
        assert_eq!(search.term, "second");
        assert!(search.messages.is_empty());
    }

    #[test]
    fn apply_search_outcome_is_a_no_op_for_an_unknown_channel() {
        let mut app = app();
        app.apply_search_outcome(SearchOutcome {
            channel: "nonexistent".to_string(),
            term: "hello".to_string(),
            messages: vec![dated_message(1, 1, PEER_ID, "hello")],
        });
        assert!(app.active().search.is_none());
    }

    #[test]
    fn slash_s_is_a_shorthand_for_search() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hello world", None),
        ));

        let action = submit(&mut app, "/s world");

        match action.expect("a successful /s returns an action") {
            InputAction::Search { channel, term } => {
                assert_eq!(channel, "general");
                assert_eq!(term, "world");
            }
            _ => panic!("expected InputAction::Search"),
        }
        let search = app
            .active()
            .search
            .as_ref()
            .expect("search should be active");
        assert_eq!(search.messages.len(), 1);
    }

    #[test]
    fn slash_s_with_no_arg_clears_an_active_filter() {
        let mut app = app();
        submit(&mut app, "/search hello");
        assert!(app.active().search.is_some());

        let action = submit(&mut app, "/s");

        assert!(action.is_none());
        assert!(app.active().search.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "search cleared");
    }

    #[test]
    fn slash_send_with_arg_returns_send_file_action_and_pushes_a_breadcrumb() {
        let mut app = app();
        let action = submit(&mut app, "/send /tmp/report.pdf");
        match action.expect("non-empty /send returns an action") {
            InputAction::SendFile { channel, path } => {
                assert_eq!(channel, "general");
                assert_eq!(path, "/tmp/report.pdf");
            }
            _ => panic!("expected InputAction::SendFile"),
        }
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "sending report.pdf...");
    }

    #[test]
    fn slash_send_strips_surrounding_double_quotes() {
        let mut app = app();
        let action = submit(&mut app, "/send \"~/Downloads/tilemap.png\"");
        match action.expect("non-empty /send returns an action") {
            InputAction::SendFile { path, .. } => {
                assert_eq!(path, "~/Downloads/tilemap.png");
            }
            _ => panic!("expected InputAction::SendFile"),
        }
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "sending tilemap.png...");
    }

    #[test]
    fn strip_quotes_strips_matching_double_or_single_quotes() {
        assert_eq!(strip_quotes("\"foo\""), "foo");
        assert_eq!(strip_quotes("'foo'"), "foo");
    }

    #[test]
    fn strip_quotes_leaves_unquoted_or_mismatched_input_alone() {
        assert_eq!(strip_quotes("foo"), "foo");
        assert_eq!(strip_quotes("\"foo"), "\"foo");
        assert_eq!(strip_quotes("'foo\""), "'foo\"");
        assert_eq!(strip_quotes("\""), "\"");
    }

    #[test]
    fn slash_save_resolves_a_known_attachment_by_hash_prefix() {
        let mut app = app();
        let message = file_message(1, PEER_ID, "report.pdf", b"report contents");
        let hash = message.attachment.as_ref().unwrap().hash;
        app.handle_net_event(NetEvent::Received("general".to_string(), message));

        let full_hash = hash.to_hex();
        let action = submit(&mut app, &format!("/save {}", &full_hash[..8]));

        match action.expect("a successful /save returns an action") {
            InputAction::SaveFile(request) => {
                assert_eq!(request.hash, hash);
                assert_eq!(request.filename, "report.pdf");
                assert_eq!(request.providers, vec![PEER_ID]);
                assert_eq!(
                    request.mode,
                    SaveMode::Manual,
                    "someone asked for it and can see it: no limit on its size"
                );
            }
            _ => panic!("expected InputAction::SaveFile"),
        }
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "saving report.pdf...");
    }

    #[test]
    fn slash_save_reports_an_error_for_an_unknown_prefix() {
        let mut app = app();
        let action = submit(&mut app, "/save ffffffff");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("no known file"), "got: {last}");
    }

    #[test]
    fn slash_save_rejects_a_non_hex_prefix() {
        let mut app = app();
        let action = submit(&mut app, "/save not-hex");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("invalid hash prefix"), "got: {last}");
    }

    #[test]
    fn resolve_attachment_ignores_duplicate_sends_of_the_same_file() {
        let mut app = app();
        let message = file_message(1, PEER_ID, "report.pdf", b"same bytes");
        let hash = message.attachment.as_ref().unwrap().hash;
        // The same file, resent (e.g. to two different channels) -- must
        // not turn into an "ambiguous prefix" error just because it shows
        // up twice.
        app.handle_net_event(NetEvent::Received("general".to_string(), message));
        let resent = file_message(2, PEER_ID_2, "report.pdf", b"same bytes");
        app.handle_net_event(NetEvent::Received("general".to_string(), resent));

        let resolved = app.resolve_attachment(&hash.to_hex()[..8]).unwrap();

        assert_eq!(resolved.0, hash);
    }

    /// Delivers `message` to `channel` as if a peer had just sent it.
    fn receive(app: &mut AppState, channel: &str, message: ChatMessage) {
        app.handle_net_event(NetEvent::Received(channel.to_string(), message));
    }

    /// What a `/save` asked to fetch, in full -- the hash, filename, and
    /// providers of its `InputAction::SaveFile` -- panicking on anything
    /// else.
    fn save_request(action: Option<InputAction>) -> (Hash, String, Vec<[u8; 32]>) {
        match action.expect("a /save with a file to save returns an action") {
            InputAction::SaveFile(request) => (request.hash, request.filename, request.providers),
            _ => panic!("expected InputAction::SaveFile"),
        }
    }

    /// The folder a `/save` asked to save into -- panicking on anything
    /// else.
    fn save_dir(action: Option<InputAction>) -> PathBuf {
        match action.expect("a /save with a file to save returns an action") {
            InputAction::SaveFile(request) => request.dir,
            _ => panic!("expected InputAction::SaveFile"),
        }
    }

    /// Which file a `/save` chose: its hash, its filename, and the sender --
    /// the first provider, the one asked before anyone else.
    fn saved_file(action: Option<InputAction>) -> (Hash, String, [u8; 32]) {
        let (hash, filename, providers) = save_request(action);
        (hash, filename, providers[0])
    }

    /// Delivers a gossip neighbor coming up on `channel` for each of `peers`,
    /// in order.
    fn peers_join(app: &mut AppState, channel: &str, peers: &[[u8; 32]]) {
        for peer in peers {
            app.handle_net_event(NetEvent::PeerJoined(channel.to_string(), *peer));
        }
    }

    #[test]
    fn bare_slash_save_saves_the_newest_file_from_another_peer() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "old.pdf", b"old contents"),
        );
        let newest = file_message(2, PEER_ID_2, "new.pdf", b"new contents");
        let newest_hash = newest.attachment.as_ref().unwrap().hash;
        receive(&mut app, "general", newest);

        let saved = saved_file(submit(&mut app, "/save"));

        assert_eq!(saved, (newest_hash, "new.pdf".to_string(), PEER_ID_2));
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "saving new.pdf...");
    }

    #[test]
    fn bare_slash_save_skips_our_own_files_even_when_they_are_newer() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "theirs.pdf", b"theirs"),
        );
        app.record_sent_message("general", file_message(2, SELF_ID, "mine.pdf", b"mine"));

        let (_, filename, sender) = saved_file(submit(&mut app, "/save"));

        assert_eq!((filename.as_str(), sender), ("theirs.pdf", PEER_ID));
    }

    #[test]
    fn bare_slash_save_looks_past_newer_lines_that_carry_no_file() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        receive(
            &mut app,
            "general",
            compose_message(PEER_ID_2, "got it, thanks", None),
        );
        app.push_system("some notice");

        let (_, filename, _) = saved_file(submit(&mut app, "/save"));

        assert_eq!(filename, "report.pdf");
    }

    #[test]
    fn bare_slash_save_is_not_narrowed_by_an_open_search() {
        // A search narrows what `/reply` can target to what is in view; the
        // file just shared belongs to the channel, not to the view.
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        receive(
            &mut app,
            "general",
            compose_message(PEER_ID_2, "hello there", None),
        );
        submit(&mut app, "/search hello");
        assert_eq!(
            app.active().visible_chat_ids().len(),
            1,
            "the search leaves only the greeting in view"
        );

        let (_, filename, _) = saved_file(submit(&mut app, "/save"));

        assert_eq!(filename, "report.pdf");
    }

    #[test]
    fn bare_slash_save_only_reads_the_active_channel() {
        let mut app = multi_channel_app(); // "random" is active
        receive(
            &mut app,
            "random",
            file_message(1, PEER_ID, "here.pdf", b"here"),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID_2, "elsewhere.pdf", b"elsewhere"),
        );

        let (_, filename, sender) = saved_file(submit(&mut app, "/save"));

        assert_eq!((filename.as_str(), sender), ("here.pdf", PEER_ID));
    }

    #[test]
    fn bare_slash_save_saves_a_file_from_one_of_our_own_other_devices() {
        // Like `ui::attachment_line`, a bare `/save` counts anyone but this
        // device as another peer: a paired sibling's file is not on this one.
        let mut app = app();
        let user_key = SecretKey::generate();
        app.load_self_user_id(*user_key.public().as_bytes());
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID),
        ));
        assert_eq!(app.display_name(&PEER_ID), "you", "PEER_ID is our sibling");
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "from-the-other-laptop.png", b"png"),
        );

        let (_, filename, sender) = saved_file(submit(&mut app, "/save"));

        assert_eq!(
            (filename.as_str(), sender),
            ("from-the-other-laptop.png", PEER_ID)
        );
    }

    #[test]
    fn bare_slash_save_says_so_when_the_channel_has_no_file() {
        let mut app = app();
        receive(&mut app, "general", compose_message(PEER_ID, "hello", None));

        let action = submit(&mut app, "/save");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("no file from another peer"), "got: {last}");
    }

    #[test]
    fn bare_slash_save_says_so_when_only_our_own_files_are_there() {
        let mut app = app();
        app.record_sent_message("general", file_message(1, SELF_ID, "mine.pdf", b"mine"));

        let action = submit(&mut app, "/save");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("no file from another peer"), "got: {last}");
    }

    #[test]
    fn bare_slash_save_says_so_when_files_were_only_shared_in_other_channels() {
        let mut app = multi_channel_app(); // "random" is active
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "elsewhere.pdf", b"elsewhere"),
        );

        let action = submit(&mut app, "/save");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("no file from another peer"), "got: {last}");
    }

    #[test]
    fn slash_save_with_a_prefix_still_reaches_an_older_file() {
        let mut app = app();
        let older = file_message(1, PEER_ID, "old.pdf", b"old contents");
        let older_hash = older.attachment.as_ref().unwrap().hash;
        receive(&mut app, "general", older);
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID_2, "new.pdf", b"new contents"),
        );

        let saved = saved_file(submit(
            &mut app,
            &format!("/save {}", &older_hash.to_hex()[..8]),
        ));

        assert_eq!(saved, (older_hash, "old.pdf".to_string(), PEER_ID));
    }

    #[test]
    fn save_asks_the_sender_first_then_online_peers_then_those_who_only_posted() {
        let mut app = app();
        // A poster who spoke before the file was shared still comes after a
        // peer who is online now: someone reachable beats someone who may
        // be long gone.
        receive(
            &mut app,
            "general",
            compose_message(PEER_ID_3, "morning", None),
        );
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        peers_join(&mut app, "general", &[PEER_ID_2]);

        let (_, _, providers) = save_request(submit(&mut app, "/save"));

        assert_eq!(providers, vec![PEER_ID, PEER_ID_2, PEER_ID_3]);
    }

    #[test]
    fn save_asks_each_peer_once() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        receive(
            &mut app,
            "general",
            compose_message(PEER_ID_2, "thanks", None),
        );
        // Both are online *and* posted, and the sender is among them.
        peers_join(&mut app, "general", &[PEER_ID_2, PEER_ID]);

        let (_, _, providers) = save_request(submit(&mut app, "/save"));

        assert_eq!(providers, vec![PEER_ID, PEER_ID_2]);
    }

    #[test]
    fn save_never_asks_this_device_but_does_ask_a_paired_sibling() {
        let mut app = app();
        let user_key = SecretKey::generate();
        app.load_self_user_id(*user_key.public().as_bytes());
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(&user_key, PEER_ID_2),
        ));
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        app.record_sent_message("general", compose_message(SELF_ID, "got it", None));
        peers_join(&mut app, "general", &[PEER_ID_2]);

        let (_, _, providers) = save_request(submit(&mut app, "/save"));

        assert_eq!(
            providers,
            vec![PEER_ID, PEER_ID_2],
            "our other device may hold the file; this one is never asked"
        );
    }

    #[test]
    fn save_of_our_own_file_still_lists_this_device_first_since_it_holds_the_file() {
        // Left out of everyone else's list, but as the sender it stays: the
        // file is already in our own blob store, so it needs no one.
        let mut app = app();
        let mine = file_message(1, SELF_ID, "mine.pdf", b"mine");
        let hash = mine.attachment.as_ref().unwrap().hash;
        app.record_sent_message("general", mine);
        peers_join(&mut app, "general", &[PEER_ID]);

        let (_, _, providers) =
            save_request(submit(&mut app, &format!("/save {}", &hash.to_hex()[..8])));

        assert_eq!(providers, vec![SELF_ID, PEER_ID]);
    }

    #[test]
    fn save_only_asks_peers_of_the_channels_that_carried_the_file() {
        let mut app = multi_channel_app(); // "random" is active
        receive(
            &mut app,
            "random",
            file_message(1, PEER_ID, "here.pdf", b"here"),
        );
        peers_join(&mut app, "random", &[PEER_ID_3]);
        // Someone who is only in #general, where the file was never shared.
        receive(
            &mut app,
            "general",
            compose_message(PEER_ID_2, "hello", None),
        );
        peers_join(&mut app, "general", &[PEER_ID_2]);

        let (_, _, providers) = save_request(submit(&mut app, "/save"));

        assert_eq!(providers, vec![PEER_ID, PEER_ID_3]);
    }

    #[test]
    fn save_asks_every_channel_the_file_was_shared_in() {
        let mut app = multi_channel_app(); // "random" is active
        let content = b"same bytes";
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", content),
        );
        receive(
            &mut app,
            "random",
            file_message(2, PEER_ID_2, "report.pdf", content),
        );
        peers_join(&mut app, "general", &[PEER_ID_3]);
        let prefix = &iroh_blobs::Hash::new(content).to_hex()[..8];

        // Resolved by prefix, the same answer whichever channel is active:
        // the first sender seen, then the peer online in #general, then the
        // one who only posted in #random.
        let (_, _, providers) = save_request(submit(&mut app, &format!("/save {prefix}")));

        assert_eq!(providers, vec![PEER_ID, PEER_ID_3, PEER_ID_2]);
    }

    #[test]
    fn save_names_the_channel_it_saves_for_so_the_copy_can_be_recorded_for_it() {
        let mut app = multi_channel_app(); // "random" is active
        let elsewhere = file_message(1, PEER_ID, "elsewhere.pdf", b"elsewhere");
        let prefix = elsewhere.attachment.as_ref().unwrap().hash.to_hex()[..8].to_string();
        receive(&mut app, "general", elsewhere);
        receive(
            &mut app,
            "random",
            file_message(2, PEER_ID, "here.pdf", b"here"),
        );

        let bare = submit(&mut app, "/save");
        let by_prefix = submit(&mut app, &format!("/save {prefix}"));

        let channel_of = |action: Option<InputAction>| match action.expect("saves a file") {
            InputAction::SaveFile(request) => request.channel,
            _ => panic!("expected InputAction::SaveFile"),
        };
        assert_eq!(channel_of(bare), "random");
        assert_eq!(
            channel_of(by_prefix),
            "general",
            "the channel the file was shared in, as its folder is"
        );
    }

    #[test]
    fn save_goes_into_the_folder_of_the_channel_the_file_was_shared_in() {
        let mut app = multi_channel_app(); // "random" is active
        app.load_files_root(PathBuf::from("/files"));
        receive(
            &mut app,
            "random",
            file_message(1, PEER_ID, "here.pdf", b"here"),
        );

        let dir = save_dir(submit(&mut app, "/save"));

        assert_eq!(dir, PathBuf::from("/files/random"));
    }

    #[test]
    fn save_by_prefix_goes_into_the_folder_of_the_channel_that_carried_the_file() {
        let mut app = multi_channel_app(); // "random" is active
        app.load_files_root(PathBuf::from("/files"));
        let message = file_message(1, PEER_ID, "elsewhere.pdf", b"elsewhere");
        let prefix = message.attachment.as_ref().unwrap().hash.to_hex()[..8].to_string();
        receive(&mut app, "general", message);

        let dir = save_dir(submit(&mut app, &format!("/save {prefix}")));

        assert_eq!(
            dir,
            PathBuf::from("/files/general"),
            "not the active channel's, which never had the file"
        );
    }

    #[test]
    fn a_file_shared_in_two_channels_is_saved_where_you_are_looking_when_it_can_be() {
        let mut app = multi_channel_app(); // "random" is active, "general" joined first
        app.load_files_root(PathBuf::from("/files"));
        let content = b"same bytes";
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", content),
        );
        receive(
            &mut app,
            "random",
            file_message(2, PEER_ID_2, "report.pdf", content),
        );
        let prefix = &iroh_blobs::Hash::new(content).to_hex()[..8];

        let from_random = save_dir(submit(&mut app, &format!("/save {prefix}")));
        app.active = 0; // general
        let from_general = save_dir(submit(&mut app, &format!("/save {prefix}")));

        assert_eq!(from_random, PathBuf::from("/files/random"));
        assert_eq!(from_general, PathBuf::from("/files/general"));
    }

    #[test]
    fn a_file_shared_in_two_other_channels_goes_to_the_first_one_joined() {
        let mut app = AppState::new(
            SELF_ID,
            vec![
                "general".to_string(),
                "random".to_string(),
                "other".to_string(),
            ],
            "other",
        );
        app.load_files_root(PathBuf::from("/files"));
        let content = b"same bytes";
        receive(
            &mut app,
            "random",
            file_message(1, PEER_ID, "report.pdf", content),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID_2, "report.pdf", content),
        );
        let prefix = &iroh_blobs::Hash::new(content).to_hex()[..8];

        let dir = save_dir(submit(&mut app, &format!("/save {prefix}")));

        assert_eq!(dir, PathBuf::from("/files/general"));
    }

    #[test]
    fn renaming_a_channel_does_not_move_its_folder() {
        let mut app = app_for_the_switcher();
        app.load_files_root(PathBuf::from("/files"));
        app.active = 2; // project-x
        receive(
            &mut app,
            "project-x",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        let before = save_dir(submit(&mut app, "/save"));

        submit(&mut app, "/rename proj");
        let after = save_dir(submit(&mut app, "/save"));

        assert_eq!(app.channel_label("project-x"), "proj");
        assert_eq!(before, PathBuf::from("/files/project-x"));
        assert_eq!(after, before, "the label changed, the stored name did not");
    }

    #[test]
    fn aliasing_a_peer_does_not_move_the_folder_of_a_conversation_with_them() {
        let mut app = app_for_the_switcher(); // `Alice` labels the DM `dm-Alice`
        app.load_files_root(PathBuf::from("/files"));
        app.active = 1;
        receive(
            &mut app,
            "dm-Alice",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        let before = save_dir(submit(&mut app, "/save"));

        submit(
            &mut app,
            &format!("/alias {} Carol", &hex_id(&PEER_ID)[..8]),
        );
        let after = save_dir(submit(&mut app, "/save"));

        assert_eq!(app.channel_label("dm-Alice"), "Carol");
        assert_eq!(before, channel_dir(Path::new("/files"), "dm-Alice"));
        assert_eq!(after, before, "the label changed, the stored name did not");
    }

    #[test]
    fn a_dm_and_a_dm_chat_save_into_folders_named_from_their_stored_names() {
        let mut app = app_for_the_switcher(); // `dm-Alice` and `p2p-08080808`
        app.load_files_root(PathBuf::from("/files"));
        receive(
            &mut app,
            "dm-Alice",
            file_message(1, PEER_ID, "a.pdf", b"from alice"),
        );
        receive(
            &mut app,
            "p2p-08080808",
            file_message(2, PEER_ID_2, "b.pdf", b"from bob"),
        );

        app.active = 1;
        let dm = save_dir(submit(&mut app, "/save"));
        app.active = 3;
        let chat = save_dir(submit(&mut app, "/save"));

        assert_eq!(dm, channel_dir(Path::new("/files"), "dm-Alice"));
        assert_eq!(chat, PathBuf::from("/files/p2p-08080808"));
        assert_ne!(dm, chat);
    }

    #[test]
    fn a_direct_folder_mapping_controls_saves_without_following_later_aliases() {
        let mut app = app_for_the_switcher();
        app.load_files_root(PathBuf::from("/files"));
        app.load_direct_folder("p2p-08080808".into(), "dm-bob-08080808".into());
        app.active = 3;
        receive(
            &mut app,
            "p2p-08080808",
            file_message(1, PEER_ID_2, "report.pdf", b"report"),
        );

        let before = save_dir(submit(&mut app, "/save"));
        submit(
            &mut app,
            &format!("/alias {} Carol", &hex_id(&PEER_ID_2)[..8]),
        );
        let after = save_dir(submit(&mut app, "/save"));

        assert_eq!(before, PathBuf::from("/files/dm-bob-08080808"));
        assert_eq!(after, before);
    }

    #[test]
    fn sync_status_reports_a_mapped_direct_folder() {
        let mut app = app_for_the_switcher();
        app.load_files_root(PathBuf::from("/files"));
        app.load_direct_folder("p2p-08080808".into(), "dm-bob-08080808".into());
        app.active = 3;
        submit(&mut app, "/sync on");

        submit(&mut app, "/sync");

        assert!(
            as_system(app.active().messages.back().unwrap()).contains("/files/dm-bob-08080808")
        );
    }

    #[test]
    fn a_bare_folder_says_where_received_files_go() {
        let mut app = app_for_the_switcher();
        app.load_files_root(PathBuf::from("/files"));
        app.active = 2; // project-x
        submit(&mut app, "/rename proj");

        let action = submit(&mut app, "/folder");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("#proj"), "names the channel, got: {last}");
        assert!(
            last.contains("/files/project-x"),
            "the channel's folder is the stored name's, got: {last}"
        );
        assert!(last.contains("under /files,"), "and the root, got: {last}");
    }

    #[test]
    fn folder_with_a_path_asks_to_move_the_files_root_and_strips_one_layer_of_quotes() {
        let mut app = app();

        for (typed, expected) in [
            ("~/Documents/leyline", "~/Documents/leyline"),
            ("\"/my files\"", "/my files"),
            ("  relative/dir  ", "relative/dir"),
        ] {
            match submit(&mut app, &format!("/folder {typed}")) {
                Some(InputAction::SetFilesRoot(path)) => assert_eq!(path, expected, "{typed}"),
                _ => panic!("expected InputAction::SetFilesRoot for {typed}"),
            }
        }
    }

    #[test]
    fn a_files_root_that_took_effect_decides_where_the_next_save_goes() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        app.load_files_root(PathBuf::from("/old"));
        assert_eq!(
            save_dir(submit(&mut app, "/save")),
            PathBuf::from("/old/general")
        );

        app.load_files_root(PathBuf::from("/new"));

        assert_eq!(
            save_dir(submit(&mut app, "/save")),
            PathBuf::from("/new/general")
        );
    }

    #[test]
    fn a_bare_maxsize_says_what_the_limit_is_and_how_to_change_it() {
        let mut app = app();

        let action = submit(&mut app, "/maxsize");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.contains("files over 500.0 MB are refused (the default)"),
            "got: {last}"
        );
        assert!(
            last.contains("/maxsize <size>") && last.contains("/maxsize off"),
            "got: {last}"
        );
    }

    #[test]
    fn autosave_limit_changes_the_automatic_request_and_refuses_off() {
        let mut app = app_with_autosave();
        app.load_contacts(alice());

        match submit(&mut app, "/autosave limit 1KB") {
            Some(InputAction::SetChannelAutoSaveLimit {
                channel,
                limit: Some(AutoSaveLimit::Bytes(1024)),
            }) if channel == "general" => {}
            _ => panic!("expected an auto-save limit action"),
        }
        assert!(
            arrive(
                &mut app,
                "general",
                file_message_of_size(1, PEER_ID, "over.bin", 1025)
            )
            .is_none()
        );

        assert!(submit(&mut app, "/autosave limit off").is_none());
        assert!(newest_notice(&app, "general").contains("auto-save limit"));
    }

    #[test]
    fn a_bare_maxsize_reports_a_chosen_limit_and_no_limit_too_without_changing_either() {
        let mut app = app();

        app.load_size_limit(SizeLimit::Bytes(1 << 20));
        assert!(submit(&mut app, "/maxsize").is_none());
        let chosen = as_system(app.active().messages.back().unwrap()).to_string();
        assert_eq!(app.size_limit(), SizeLimit::Bytes(1 << 20));

        app.load_size_limit(SizeLimit::Off);
        assert!(submit(&mut app, "/maxsize").is_none());
        let off = as_system(app.active().messages.back().unwrap()).to_string();
        assert_eq!(app.size_limit(), SizeLimit::Off);

        assert!(
            chosen.contains("files over 1.0 MB are refused (set with /maxsize)"),
            "got: {chosen}"
        );
        assert!(
            chosen.contains("/maxsize default restores the 500.0 MB default"),
            "got: {chosen}"
        );
        assert!(
            off.contains("there is no limit on the size of a file you share"),
            "got: {off}"
        );
    }

    #[test]
    fn maxsize_with_a_size_applies_it_at_once_and_asks_for_it_to_be_remembered() {
        let mut app = app();

        match submit(&mut app, "/maxsize 2GB").expect("a limit that took effect is remembered") {
            InputAction::SetSizeLimit(limit) => assert_eq!(limit, SizeLimit::Bytes(2 << 30)),
            _ => panic!("expected InputAction::SetSizeLimit"),
        }

        assert_eq!(app.size_limit(), SizeLimit::Bytes(2 << 30));
        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.contains("files over 2.0 GB are refused (set with /maxsize)"),
            "got: {last}"
        );
    }

    #[test]
    fn maxsize_off_lifts_the_limit_and_default_restores_it() {
        let mut app = app();

        match submit(&mut app, "/maxsize off") {
            Some(InputAction::SetSizeLimit(limit)) => assert_eq!(limit, SizeLimit::Off),
            _ => panic!("expected InputAction::SetSizeLimit"),
        }
        assert_eq!(app.size_limit(), SizeLimit::Off);
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("there is no limit"), "got: {last}");

        match submit(&mut app, "/maxsize default") {
            Some(InputAction::SetSizeLimit(limit)) => assert_eq!(limit, SizeLimit::Default),
            _ => panic!("expected InputAction::SetSizeLimit"),
        }
        assert_eq!(app.size_limit(), SizeLimit::Default);
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("(the default)"), "got: {last}");
    }

    #[test]
    fn a_maxsize_that_is_not_a_size_is_refused_and_changes_nothing() {
        let mut app = app();
        app.load_size_limit(SizeLimit::Bytes(1 << 20));

        for typed in ["banana", "5", "0MB", "2XB"] {
            let action = submit(&mut app, &format!("/maxsize {typed}"));

            assert!(action.is_none(), "{typed}: nothing to remember");
            assert_eq!(
                app.size_limit(),
                SizeLimit::Bytes(1 << 20),
                "{typed}: unchanged"
            );
            let last = as_system(app.active().messages.back().unwrap());
            assert!(last.contains(&format!("'{typed}'")), "{typed}: got {last}");
        }
    }

    #[test]
    fn the_size_limit_is_the_default_until_one_is_loaded() {
        let mut app = app();
        assert_eq!(app.size_limit(), SizeLimit::Default);

        app.load_size_limit(SizeLimit::Off);

        assert_eq!(app.size_limit(), SizeLimit::Off);
    }

    #[test]
    fn help_lists_maxsize() {
        let mut app = app();

        submit(&mut app, "/help");

        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("/maxsize"), "got: {last}");
    }

    #[test]
    fn record_sent_message_places_the_message_without_marking_unread() {
        let mut app = multi_channel_app();
        app.active = 1; // "random" is active; "general" is not
        let message = file_message(1, SELF_ID, "report.pdf", b"contents");

        app.record_sent_message("general", message.clone());

        assert_eq!(as_chat(app.channels[0].messages.back().unwrap()), &message);
        assert!(
            app.channels[0].unread_count == 0,
            "a self-sent file shouldn't flag its channel unread"
        );
    }

    #[test]
    fn record_sent_message_dedupes_by_id() {
        let mut app = app();
        let message = file_message(1, SELF_ID, "report.pdf", b"contents");
        app.record_sent_message("general", message.clone());

        let before = app.active().messages.len();
        app.record_sent_message("general", message);

        assert_eq!(app.active().messages.len(), before);
    }

    #[test]
    fn file_send_failed_and_save_failed_are_a_no_op_here() {
        // Both are handled in main.rs instead (which also logs them via
        // tracing::warn!, unlike a plain push_system) -- see
        // handle_net_event's doc comment.
        let mut app = app();
        let before = app.active().messages.len();

        app.handle_net_event(NetEvent::FileSendFailed {
            path: "/tmp/missing.pdf".to_string(),
            error: "no such file".to_string(),
        });
        app.handle_net_event(NetEvent::FileSaveFailed {
            channel: "general".to_string(),
            hash: iroh_blobs::Hash::new(b"report"),
            filename: "report.pdf".to_string(),
            error: "boom".to_string(),
            mode: SaveMode::Manual,
        });

        assert_eq!(app.active().messages.len(), before);
    }

    #[test]
    fn file_saved_pushes_a_system_notice_with_the_final_path() {
        let mut app = app();
        app.handle_net_event(NetEvent::FileSaved {
            channel: "general".to_string(),
            hash: iroh_blobs::Hash::new(b"report"),
            filename: "report.pdf".to_string(),
            path: std::path::PathBuf::from("/home/alice/Downloads/leyline/report.pdf"),
            mode: SaveMode::Manual,
        });
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(
            last,
            "saved report.pdf to /home/alice/Downloads/leyline/report.pdf"
        );
    }

    #[test]
    fn search_matches_an_attachments_filename() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            file_message(1, PEER_ID, "vacation-photo.png", b"bytes"),
        ));

        submit(&mut app, "/search vacation");

        let search = app
            .active()
            .search
            .as_ref()
            .expect("search should be active");
        assert_eq!(search.messages.len(), 1);
    }

    fn ctrl_r(app: &mut AppState) -> Option<InputAction> {
        app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))
    }

    #[test]
    fn find_message_resolves_a_message_still_in_the_transcript() {
        let mut app = app();
        let parent = compose_message(PEER_ID, "original", None);
        let parent_id = parent.id;
        app.handle_net_event(NetEvent::Received("general".to_string(), parent));
        let reply = reply_message(1, 999, PEER_ID_2, "sounds good", parent_id);
        app.handle_net_event(NetEvent::Received("general".to_string(), reply));

        let found = app
            .active()
            .find_message(parent_id)
            .expect("parent must resolve");
        assert_eq!(found.text, "original");
    }

    #[test]
    fn find_message_returns_none_for_an_unknown_id() {
        let app = app();
        assert!(app.active().find_message(12345).is_none());
    }

    #[test]
    fn ctrl_r_with_no_messages_pushes_a_notice_and_does_not_enter_picking() {
        let mut app = app();
        ctrl_r(&mut app);
        assert!(app.picking_reply.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "no messages to reply to");
    }

    #[test]
    fn ctrl_r_selects_the_newest_message() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "first", None),
        ));
        let newest = compose_message(PEER_ID, "second", None);
        let newest_id = newest.id;
        app.handle_net_event(NetEvent::Received("general".to_string(), newest));

        ctrl_r(&mut app);

        assert_eq!(app.picking_reply, Some(newest_id));
    }

    #[test]
    fn ctrl_r_picks_from_active_search_results_when_a_search_is_running() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "apple", None),
        ));
        let matching = compose_message(PEER_ID, "banana split", None);
        let matching_id = matching.id;
        app.handle_net_event(NetEvent::Received("general".to_string(), matching));
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "cherry", None),
        ));

        submit(&mut app, "/search banana");
        ctrl_r(&mut app);

        assert_eq!(
            app.picking_reply,
            Some(matching_id),
            "picking must be scoped to the active search's results"
        );
    }

    #[test]
    fn up_and_down_move_the_pick_older_and_newer_and_clamp_at_both_ends() {
        let mut app = app();
        let first = compose_message(PEER_ID, "first", None);
        let second = compose_message(PEER_ID, "second", None);
        let (first_id, second_id) = (first.id, second.id);
        app.handle_net_event(NetEvent::Received("general".to_string(), first));
        app.handle_net_event(NetEvent::Received("general".to_string(), second));

        ctrl_r(&mut app);
        assert_eq!(app.picking_reply, Some(second_id));

        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.picking_reply, Some(first_id));
        app.handle_key(key(KeyCode::Up));
        assert_eq!(
            app.picking_reply,
            Some(first_id),
            "must clamp at the oldest message"
        );

        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.picking_reply, Some(second_id));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(
            app.picking_reply,
            Some(second_id),
            "must clamp at the newest message"
        );
    }

    #[test]
    fn enter_confirms_the_pick_and_exits_picking() {
        let mut app = app();
        let message = compose_message(PEER_ID, "hi", None);
        let id = message.id;
        app.handle_net_event(NetEvent::Received("general".to_string(), message));

        ctrl_r(&mut app);
        app.handle_key(key(KeyCode::Enter));

        assert!(app.picking_reply.is_none());
        assert_eq!(app.replying_to, Some(id));
    }

    #[test]
    fn esc_while_picking_cancels_without_quitting() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hi", None),
        ));
        ctrl_r(&mut app);

        app.handle_key(key(KeyCode::Esc));

        assert!(app.picking_reply.is_none());
        assert!(app.replying_to.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn ctrl_c_while_picking_still_quits() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hi", None),
        ));
        ctrl_r(&mut app);

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(app.should_quit);
    }

    #[test]
    fn esc_with_an_armed_reply_clears_it_without_quitting() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hi", None),
        ));
        ctrl_r(&mut app);
        app.handle_key(key(KeyCode::Enter));
        assert!(app.replying_to.is_some());

        app.handle_key(key(KeyCode::Esc));

        assert!(app.replying_to.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn sending_while_armed_attaches_reply_to_and_clears_armed_state() {
        let mut app = app();
        let parent = compose_message(PEER_ID, "original", None);
        let parent_id = parent.id;
        app.handle_net_event(NetEvent::Received("general".to_string(), parent));
        app.replying_to = Some(parent_id);

        let action = submit(&mut app, "sounds good");

        match action.expect("a non-empty send returns an action") {
            InputAction::Send(_, message) => assert_eq!(message.reply_to, Some(parent_id)),
            _ => panic!("expected InputAction::Send"),
        }
        assert!(app.replying_to.is_none());
    }

    #[test]
    fn running_a_command_while_armed_does_not_clear_it() {
        let mut app = app();
        app.handle_net_event(NetEvent::Received(
            "general".to_string(),
            compose_message(PEER_ID, "hi", None),
        ));
        ctrl_r(&mut app);
        app.handle_key(key(KeyCode::Enter));
        assert!(app.replying_to.is_some());

        submit(&mut app, "/hints");

        assert!(
            app.replying_to.is_some(),
            "an unrelated command must not clear an armed reply"
        );
    }

    #[test]
    fn switch_channel_clears_an_armed_reply() {
        let mut app = multi_channel_app();
        app.replying_to = Some(2);

        app.handle_key(key(KeyCode::Tab));

        assert!(app.replying_to.is_none());
    }

    #[test]
    fn switch_channel_directly_clears_picking_and_armed_reply() {
        // `switch_channel` itself must clear all three, even though `Tab`
        // can't actually reach it while picking in practice -- `handle_key`
        // routes every key through `handle_picking_key`/
        // `handle_thread_picking_key` first in that state (see
        // `esc_while_picking_cancels_without_quitting` and friends),
        // neither of which has a `Tab` arm of its own. Calling the method
        // directly still documents/guards the invariant.
        let mut app = multi_channel_app();
        app.picking_reply = Some(1);
        app.picking_thread = Some(3);
        app.replying_to = Some(2);

        app.switch_channel(1);

        assert!(app.picking_reply.is_none());
        assert!(app.picking_thread.is_none());
        assert!(app.replying_to.is_none());
    }

    #[test]
    fn remove_channel_of_the_active_one_clears_picking_and_armed_reply() {
        let mut app = multi_channel_app();
        assert_eq!(app.active().name, "random");
        app.picking_reply = Some(1);
        app.picking_thread = Some(3);
        app.replying_to = Some(2);

        app.remove_channel("random");

        assert!(app.picking_reply.is_none());
        assert!(app.picking_thread.is_none());
        assert!(app.replying_to.is_none());
    }

    #[test]
    fn remove_channel_of_a_different_tab_preserves_picking_and_armed_reply() {
        let mut app = multi_channel_app();
        assert_eq!(app.active().name, "random");
        app.picking_reply = Some(1);
        app.picking_thread = Some(3);
        app.replying_to = Some(2);

        app.remove_channel("general");

        assert_eq!(app.picking_reply, Some(1));
        assert_eq!(app.picking_thread, Some(3));
        assert_eq!(app.replying_to, Some(2));
    }

    #[test]
    fn joined_event_clears_picking_and_armed_reply() {
        let mut app = app();
        app.picking_reply = Some(1);
        app.picking_thread = Some(3);
        app.replying_to = Some(2);

        app.handle_net_event(NetEvent::Joined("project-x".to_string()));

        assert!(app.picking_reply.is_none());
        assert!(app.picking_thread.is_none());
        assert!(app.replying_to.is_none());
    }

    #[test]
    fn slash_reply_with_no_messages_reports_an_error() {
        let mut app = app();
        let action = submit(&mut app, "/reply");
        assert!(action.is_none());
        assert!(app.replying_to.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "no messages to reply to");
    }

    #[test]
    fn slash_reply_with_no_text_arms_the_most_recent_message_and_pushes_a_notice() {
        let mut app = app();
        let message = compose_message(PEER_ID, "original", None);
        let id = message.id;
        app.handle_net_event(NetEvent::Received("general".to_string(), message));

        let action = submit(&mut app, "/reply");

        assert!(action.is_none());
        assert_eq!(app.replying_to, Some(id));
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("replying to"), "got: {last}");
    }

    #[test]
    fn slash_reply_with_text_arms_and_sends_in_one_step() {
        let mut app = app();
        let message = compose_message(PEER_ID, "original", None);
        let id = message.id;
        app.handle_net_event(NetEvent::Received("general".to_string(), message));

        let action = submit(&mut app, "/reply sounds good");

        match action.expect("/reply <text> returns a send action") {
            InputAction::Send(channel, sent) => {
                assert_eq!(channel, "general");
                assert_eq!(sent.text, "sounds good");
                assert_eq!(sent.reply_to, Some(id));
            }
            _ => panic!("expected InputAction::Send"),
        }
        assert!(app.replying_to.is_none());
    }

    #[test]
    fn thread_command_opens_the_newest_messages_chain() {
        let mut app = app();
        app.load_history(
            "general",
            vec![
                dated_message(100, 1, PEER_ID, "root"),
                reply_message(200, 2, SELF_ID, "reply", 1),
            ],
        );

        let action = submit(&mut app, "/thread");

        assert!(action.is_none());
        let thread = app
            .active()
            .thread
            .as_ref()
            .expect("thread view must be active");
        assert_eq!(
            thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(
            thread.origin.id, 2,
            "a bare /thread targets the newest visible message"
        );
    }

    #[test]
    fn thread_command_reports_when_there_are_no_messages() {
        let mut app = app();
        let action = submit(&mut app, "/thread");
        assert!(action.is_none());
        assert!(app.active().thread.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "no messages to view a thread for");
    }

    #[test]
    fn thread_command_closes_an_active_thread_view() {
        let mut app = app();
        app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
        submit(&mut app, "/thread");
        assert!(app.active().thread.is_some());

        let action = submit(&mut app, "/thread");

        assert!(action.is_none());
        assert!(app.active().thread.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "thread view closed");
    }

    #[test]
    fn esc_closes_an_active_thread_view_without_quitting() {
        let mut app = app();
        app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
        submit(&mut app, "/thread");

        app.handle_key(key(KeyCode::Esc));

        assert!(app.active().thread.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn ctrl_t_pick_then_enter_opens_the_picked_messages_thread() {
        let mut app = app();
        app.load_history(
            "general",
            vec![
                dated_message(100, 1, PEER_ID, "first"),
                dated_message(200, 2, PEER_ID, "second"),
            ],
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        assert_eq!(
            app.picking_thread,
            Some(2),
            "Ctrl+T starts on the newest message"
        );
        app.handle_key(key(KeyCode::Up));
        app.handle_key(key(KeyCode::Enter));

        assert!(app.picking_thread.is_none());
        let thread = app
            .active()
            .thread
            .as_ref()
            .expect("thread view must be active");
        assert_eq!(thread.origin.id, 1);
    }

    #[test]
    fn esc_while_thread_picking_cancels_without_opening_a_thread() {
        let mut app = app();
        app.load_history("general", vec![dated_message(100, 1, PEER_ID, "hi")]);

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        app.handle_key(key(KeyCode::Esc));

        assert!(app.picking_thread.is_none());
        assert!(app.active().thread.is_none());
        assert!(!app.should_quit);
    }

    #[test]
    fn opening_a_thread_clears_an_active_search() {
        let mut app = app();
        app.load_history(
            "general",
            vec![
                dated_message(100, 1, PEER_ID, "hello world"),
                dated_message(200, 2, PEER_ID, "unrelated"),
            ],
        );
        submit(&mut app, "/search hello");
        assert!(app.active().search.is_some());

        submit(&mut app, "/thread");

        assert!(app.active().search.is_none());
        assert!(app.active().thread.is_some());
    }

    #[test]
    fn starting_a_search_clears_an_active_thread() {
        let mut app = app();
        app.load_history(
            "general",
            vec![dated_message(100, 1, PEER_ID, "hello world")],
        );
        submit(&mut app, "/thread");
        assert!(app.active().thread.is_some());

        submit(&mut app, "/search hello");

        assert!(app.active().thread.is_none());
        assert!(app.active().search.is_some());
    }

    #[test]
    fn thread_excludes_a_sibling_reply_to_the_same_parent() {
        let mut app = app();
        app.load_history(
            "general",
            vec![
                dated_message(100, 1, PEER_ID, "parent"),
                reply_message(200, 2, PEER_ID, "origin", 1),
                reply_message(300, 3, PEER_ID, "sibling", 1),
            ],
        );

        app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        app.handle_key(key(KeyCode::Up)); // move highlight from id 3 to id 2
        app.handle_key(key(KeyCode::Enter));

        let thread = app
            .active()
            .thread
            .as_ref()
            .expect("thread view must be active");
        assert_eq!(
            thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2],
            "a sibling reply to the same parent must not be pulled in"
        );
    }

    #[test]
    fn sending_a_plain_message_while_viewing_a_thread_replies_within_it() {
        let mut app = app();
        app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
        submit(&mut app, "/thread");

        let action = submit(&mut app, "joining the thread");

        match action.expect("a non-empty send returns an action") {
            InputAction::Send(_, message) => {
                assert_eq!(
                    message.reply_to,
                    Some(1),
                    "a plain message sent while viewing a thread must join it"
                );
            }
            _ => panic!("expected InputAction::Send"),
        }
    }

    #[test]
    fn explicit_reply_target_wins_over_the_automatic_thread_target() {
        let mut app = app();
        app.load_history(
            "general",
            vec![
                dated_message(100, 1, PEER_ID, "root"),
                dated_message(200, 2, PEER_ID, "another message"),
            ],
        );
        submit(&mut app, "/thread"); // opens around the newest message (id 2)
        app.replying_to = Some(1); // explicitly armed, e.g. via Ctrl+R or /reply

        let action = submit(&mut app, "explicit reply");

        match action.expect("a non-empty send returns an action") {
            InputAction::Send(_, message) => {
                assert_eq!(
                    message.reply_to,
                    Some(1),
                    "an explicitly armed reply target must win over the thread's implicit one"
                );
            }
            _ => panic!("expected InputAction::Send"),
        }
    }

    #[test]
    fn sending_a_plain_message_without_a_thread_view_has_no_reply_to() {
        let mut app = app();

        let action = submit(&mut app, "just chatting");

        match action.expect("a non-empty send returns an action") {
            InputAction::Send(_, message) => assert_eq!(message.reply_to, None),
            _ => panic!("expected InputAction::Send"),
        }
    }

    #[test]
    fn sending_a_plain_message_while_viewing_a_thread_shows_up_immediately() {
        let mut app = app();
        app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
        submit(&mut app, "/thread");

        submit(&mut app, "joining the thread");

        let thread = app
            .active()
            .thread
            .as_ref()
            .expect("thread view must still be active");
        assert_eq!(
            thread.messages.len(),
            2,
            "the just-sent reply must show up without re-running /thread"
        );
        assert!(
            thread
                .messages
                .iter()
                .any(|m| m.text == "joining the thread"),
            "got: {:?}",
            thread.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_received_reply_to_a_message_in_the_open_thread_appears_immediately() {
        let mut app = app();
        app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
        submit(&mut app, "/thread");

        let reply = reply_message(200, 2, PEER_ID_2, "from a peer", 1);
        app.handle_net_event(NetEvent::Received("general".to_string(), reply));

        let thread = app
            .active()
            .thread
            .as_ref()
            .expect("thread view must still be active");
        assert_eq!(
            thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn an_unrelated_received_message_does_not_join_the_open_thread() {
        let mut app = app();
        app.load_history("general", vec![dated_message(100, 1, PEER_ID, "root")]);
        submit(&mut app, "/thread");

        let unrelated = dated_message(200, 2, PEER_ID_2, "different topic");
        app.handle_net_event(NetEvent::Received("general".to_string(), unrelated));

        let thread = app
            .active()
            .thread
            .as_ref()
            .expect("thread view must still be active");
        assert_eq!(
            thread.messages.iter().map(|m| m.id).collect::<Vec<_>>(),
            vec![1],
            "a message with no reply_to into the thread must not join it"
        );
    }

    #[test]
    fn msg_usage_error_when_target_is_missing() {
        let mut app = app();
        let action = submit(&mut app, "/msg");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"));
    }

    #[test]
    fn msg_reports_invalid_hex_for_a_non_hex_unaliased_target() {
        let mut app = app();
        let action = submit(&mut app, "/msg carol");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("invalid hex prefix"), "got: {last}");
    }

    #[test]
    fn msg_reports_no_known_peer_for_an_unknown_hex_prefix() {
        let mut app = app();
        let action = submit(&mut app, "/msg ffffffff");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("no known peer"), "got: {last}");
    }

    #[test]
    fn msg_reports_an_error_for_an_ambiguous_alias() {
        let mut app = app();
        app.load_contacts(HashMap::from([
            (PEER_ID, "Alice".to_string()),
            (PEER_ID_2, "Alice".to_string()),
        ]));

        let action = submit(&mut app, "/msg alice");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("multiple aliased peers"), "got: {last}");
    }

    #[test]
    fn msg_by_alias_starts_a_new_dm_when_none_exists() {
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/msg alice");

        match action.expect("/msg with no existing DM returns an action") {
            InputAction::StartDm {
                peer,
                channel,
                text,
            } => {
                assert_eq!(peer, PEER_ID);
                assert_eq!(channel, "dm-Alice");
                assert!(text.is_none());
            }
            _ => panic!("expected InputAction::StartDm"),
        }
    }

    #[test]
    fn msg_target_matching_is_case_insensitive() {
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/msg ALICE");

        match action.expect("a case-insensitive alias match should still work") {
            InputAction::StartDm { peer, .. } => assert_eq!(peer, PEER_ID),
            _ => panic!("expected InputAction::StartDm"),
        }
    }

    #[test]
    fn msg_by_hex_prefix_starts_a_new_dm_named_from_the_hex_prefix() {
        let mut app = app();
        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));

        let full_id = hex_id(&PEER_ID);
        let action = submit(&mut app, &format!("/msg {}", &full_id[..8]));

        match action.expect("/msg with a hex prefix returns an action") {
            InputAction::StartDm { peer, channel, .. } => {
                assert_eq!(peer, PEER_ID);
                assert_eq!(channel, format!("dm-{}", hex_prefix(&PEER_ID)));
            }
            _ => panic!("expected InputAction::StartDm"),
        }
    }

    #[test]
    fn msg_with_inline_text_queues_it_on_the_new_dm_action() {
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/msg alice hey there");

        match action.expect("a successful /msg returns an action") {
            InputAction::StartDm { text, .. } => {
                assert_eq!(text, Some("hey there".to_string()));
            }
            _ => panic!("expected InputAction::StartDm"),
        }
    }

    #[test]
    fn msg_disambiguates_the_channel_name_when_already_taken() {
        let mut app = AppState::new(
            SELF_ID,
            vec!["general".to_string(), "dm-Alice".to_string()],
            "general",
        );
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/msg alice");

        match action.expect("a successful /msg returns an action") {
            InputAction::StartDm { channel, .. } => assert_eq!(channel, "dm-Alice-2"),
            _ => panic!("expected InputAction::StartDm"),
        }
    }

    #[test]
    fn msg_switches_to_an_already_joined_dm_channel_instead_of_creating_another() {
        let mut app = AppState::new(
            SELF_ID,
            vec!["general".to_string(), "dm-Alice".to_string()],
            "general",
        );
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        app.load_dm_registry(HashMap::from([(PEER_ID, "dm-Alice".to_string())]));

        let action = submit(&mut app, "/msg alice");

        assert!(action.is_none(), "no inline text means nothing to send");
        assert_eq!(app.active().name, "dm-Alice");
    }

    #[test]
    fn msg_switches_to_an_existing_dm_and_sends_inline_text() {
        let mut app = AppState::new(
            SELF_ID,
            vec!["general".to_string(), "dm-Alice".to_string()],
            "general",
        );
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        app.load_dm_registry(HashMap::from([(PEER_ID, "dm-Alice".to_string())]));

        let action = submit(&mut app, "/msg alice hello again");

        assert_eq!(app.active().name, "dm-Alice");
        match action.expect("inline text should send immediately") {
            InputAction::Send(channel, message) => {
                assert_eq!(channel, "dm-Alice");
                assert_eq!(message.text, "hello again");
            }
            _ => panic!("expected InputAction::Send"),
        }
    }

    #[test]
    fn msg_creates_a_fresh_dm_when_the_recorded_channel_is_no_longer_joined() {
        let mut app = app(); // only "general" is joined
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        app.load_dm_registry(HashMap::from([(PEER_ID, "dm-Alice".to_string())]));

        let action = submit(&mut app, "/msg alice");

        match action.expect("should fall back to creating a fresh DM") {
            InputAction::StartDm { channel, .. } => assert_eq!(channel, "dm-Alice"),
            _ => panic!("expected InputAction::StartDm"),
        }
    }

    #[test]
    fn dm_joined_activates_the_channel_and_a_later_msg_reuses_it() {
        let mut app = app();

        let result = app.handle_net_event(NetEvent::DmJoined {
            channel: "dm-Alice".to_string(),
            peer: PEER_ID,
        });

        assert!(result.is_none());
        assert_eq!(app.active().name, "dm-Alice");
        assert_eq!(app.channels.len(), 2);

        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        let action = submit(&mut app, "/msg alice");
        assert!(action.is_none(), "should just switch, nothing to send");
        assert_eq!(app.active().name, "dm-Alice");
        assert_eq!(app.channels.len(), 2, "must not create a second DM channel");
    }

    #[test]
    fn record_dm_channel_lets_a_later_msg_reuse_it() {
        let mut app = AppState::new(
            SELF_ID,
            vec!["general".to_string(), "dm-Alice".to_string()],
            "dm-Alice",
        );
        app.record_dm_channel(PEER_ID, "dm-Alice".to_string());
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/msg alice");

        assert!(action.is_none());
        assert_eq!(app.active().name, "dm-Alice");
    }

    #[test]
    fn dm_reports_usage_with_no_argument() {
        let mut app = app();
        let action = submit(&mut app, "/dm");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"), "got: {last}");
    }

    #[test]
    fn dm_reports_no_known_peer_for_an_unknown_short_hex_prefix() {
        let mut app = app();
        let action = submit(&mut app, "/dm ffffffff");
        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("no known peer"), "got: {last}");
    }

    #[test]
    fn dm_accepts_a_full_endpoint_id_even_if_never_seen() {
        // The whole point of /dm: a peer never observed in any channel
        // and never /alias-ed can still be reached directly, as long as
        // their full endpoint id is known -- unlike /msg, which has
        // nothing but `known_ids()` to resolve a hex prefix against.
        let mut app = app();
        let full_id = hex_id(&PEER_ID);

        let action = submit(&mut app, &format!("/dm {full_id}"));

        match action.expect("/dm with a full endpoint id returns an action") {
            InputAction::OpenDirect { peer, text } => {
                assert_eq!(peer, PEER_ID);
                assert!(text.is_none());
            }
            _ => panic!("expected InputAction::OpenDirect"),
        }
    }

    #[test]
    fn dm_by_alias_starts_a_new_conversation_when_none_exists() {
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/dm alice");

        match action.expect("/dm with no existing conversation returns an action") {
            InputAction::OpenDirect { peer, text } => {
                assert_eq!(peer, PEER_ID);
                assert!(text.is_none());
            }
            _ => panic!("expected InputAction::OpenDirect"),
        }
    }

    #[test]
    fn dm_with_inline_text_queues_it_on_the_open_direct_action() {
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/dm alice hey there");

        match action.expect("a successful /dm returns an action") {
            InputAction::OpenDirect { text, .. } => {
                assert_eq!(text, Some("hey there".to_string()));
            }
            _ => panic!("expected InputAction::OpenDirect"),
        }
    }

    #[test]
    fn dm_refuses_to_message_yourself() {
        let mut app = app();
        let full_id = hex_id(&SELF_ID);

        let action = submit(&mut app, &format!("/dm {full_id}"));

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("yourself"), "got: {last}");
    }

    #[test]
    fn dm_switches_to_an_already_open_direct_chat_instead_of_starting_another() {
        let mut app = app();
        app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        app.active = 0; // switch away, so /dm has to switch back

        let action = submit(&mut app, "/dm alice");

        assert!(action.is_none(), "no inline text means nothing to send");
        assert_eq!(app.active().name, "p2p-alice");
    }

    #[test]
    fn dm_switches_to_an_existing_direct_chat_and_sends_inline_text() {
        let mut app = app();
        app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        app.active = 0;

        let action = submit(&mut app, "/dm alice hello again");

        assert_eq!(app.active().name, "p2p-alice");
        match action.expect("inline text should send immediately") {
            InputAction::Send(channel, message) => {
                assert_eq!(channel, "p2p-alice");
                assert_eq!(message.text, "hello again");
            }
            _ => panic!("expected InputAction::Send"),
        }
    }

    #[test]
    fn activate_direct_channel_reports_whether_the_tab_is_new() {
        let mut app = app();
        assert!(app.activate_direct_channel("p2p-alice".to_string(), PEER_ID));
        assert!(
            !app.activate_direct_channel("p2p-alice".to_string(), PEER_ID),
            "an already-open point-to-point tab is not new"
        );
    }

    #[test]
    fn activate_direct_channel_switches_focus_and_records_the_peer() {
        let mut app = app();
        app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);
        assert_eq!(app.active().name, "p2p-alice");
        assert_eq!(app.direct_peer_for("p2p-alice"), Some(PEER_ID));
    }

    #[test]
    fn background_direct_channel_does_not_switch_focus() {
        let mut app = app();
        assert_eq!(app.active().name, "general");

        let is_new = app.background_direct_channel("p2p-alice".to_string(), PEER_ID);

        assert!(is_new);
        assert_eq!(
            app.active().name,
            "general",
            "a background direct channel must not steal focus"
        );
        assert!(app.channels.iter().any(|c| c.name == "p2p-alice"));
        assert_eq!(app.direct_peer_for("p2p-alice"), Some(PEER_ID));
    }

    #[test]
    fn background_direct_channel_on_an_already_known_peer_does_not_duplicate() {
        let mut app = app();
        app.background_direct_channel("p2p-alice".to_string(), PEER_ID);
        let is_new = app.background_direct_channel("p2p-alice".to_string(), PEER_ID);
        assert!(!is_new);
        assert_eq!(app.channels.len(), 2, "must not duplicate an existing tab");
    }

    #[test]
    fn remove_channel_forgets_a_point_to_point_association() {
        let mut app = app();
        app.background_direct_channel("p2p-07070707".to_string(), PEER_ID);

        app.remove_channel("p2p-07070707");

        assert_eq!(app.direct_peer_for("p2p-07070707"), None);
    }

    #[test]
    fn remove_channel_keeps_other_channels_associations() {
        let mut app = app();
        app.background_direct_channel("p2p-07070707".to_string(), PEER_ID);
        app.background_direct_channel("p2p-08080808".to_string(), PEER_ID_2);

        app.remove_channel("p2p-07070707");

        assert_eq!(app.direct_peer_for("p2p-08080808"), Some(PEER_ID_2));
    }

    #[test]
    fn remove_channel_forgets_a_dms_association_so_a_reused_name_is_a_group() {
        let mut app = app_with_dm("dm-Alice", PEER_ID);
        app.remove_channel("dm-Alice");

        // The same name later reused for an ordinary channel must not
        // inherit the old DM's classification.
        app.handle_net_event(NetEvent::Joined("dm-Alice".to_string()));

        assert_eq!(app.channel_kind("dm-Alice"), ChannelKind::Group);
    }

    #[test]
    fn the_left_notice_names_a_dm_by_its_heading() {
        let mut app = app_with_dm("dm-07070707", PEER_ID);
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        app.remove_channel("dm-07070707");

        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "left @Alice");
    }

    #[test]
    fn the_left_notice_names_a_point_to_point_chat_by_its_heading() {
        let mut app = app();
        app.background_direct_channel("p2p-07070707".to_string(), PEER_ID);
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        app.remove_channel("p2p-07070707");

        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "left ⚡Alice");
    }

    #[test]
    fn invite_is_refused_on_a_point_to_point_channel() {
        let mut app = app();
        app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);

        let action = submit(&mut app, "/invite");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("point-to-point"), "got: {last}");
    }

    /// An app with `general` (active) plus a `/msg` DM tab named `name`
    /// associated with `peer` -- the shape the channel-label tests below
    /// all start from.
    fn app_with_dm(name: &str, peer: [u8; 32]) -> AppState {
        let mut app = AppState::new(
            SELF_ID,
            vec!["general".to_string(), name.to_string()],
            "general",
        );
        app.record_dm_channel(peer, name.to_string());
        app
    }

    #[test]
    fn an_ordinary_channel_is_a_group() {
        assert_eq!(app().channel_kind("general"), ChannelKind::Group);
    }

    #[test]
    fn a_msg_dm_channel_is_classified_with_its_peer() {
        let app = app_with_dm("dm-Alice", PEER_ID);
        assert_eq!(
            app.channel_kind("dm-Alice"),
            ChannelKind::GossipDm { peer: PEER_ID }
        );
    }

    #[test]
    fn a_point_to_point_channel_is_classified_with_its_peer() {
        let mut app = app();
        app.background_direct_channel("p2p-07070707".to_string(), PEER_ID);
        assert_eq!(
            app.channel_kind("p2p-07070707"),
            ChannelKind::Direct { peer: PEER_ID }
        );
    }

    #[test]
    fn a_channel_that_only_looks_like_a_dm_is_still_a_group() {
        // Classification comes from the recorded association, never from
        // the shape of the name.
        let mut app = app();
        app.handle_net_event(NetEvent::Joined("dm-Alice".to_string()));

        assert_eq!(app.channel_kind("dm-Alice"), ChannelKind::Group);
        assert_eq!(app.channel_label("dm-Alice"), "dm-Alice");
        assert_eq!(app.channel_heading("dm-Alice"), "#dm-Alice");
    }

    #[test]
    fn a_group_channels_label_is_its_stored_name() {
        let mut app = multi_channel_app();
        // A peer aliased the same as a group channel must not leak into
        // that channel's label.
        app.load_contacts(HashMap::from([(PEER_ID, "random".to_string())]));

        assert_eq!(app.channel_label("general"), "general");
        assert_eq!(app.channel_label("random"), "random");
    }

    #[test]
    fn a_dm_label_follows_a_later_alias() {
        let mut app = app_with_dm("dm-07070707", PEER_ID);
        assert_eq!(app.channel_label("dm-07070707"), "07070707");

        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        submit(
            &mut app,
            &format!("/alias {} Alice", &hex_id(&PEER_ID)[..8]),
        );

        assert_eq!(app.channel_label("dm-07070707"), "Alice");
    }

    #[test]
    fn a_point_to_point_label_follows_a_later_alias() {
        let raw = crate::direct::channel_name(&PEER_ID);
        let mut app = app();
        app.background_direct_channel(raw.clone(), PEER_ID);
        assert_eq!(app.channel_label(&raw), "07070707");

        app.handle_net_event(NetEvent::PeerJoined("general".to_string(), PEER_ID));
        submit(
            &mut app,
            &format!("/alias {} Alice", &hex_id(&PEER_ID)[..8]),
        );

        assert_eq!(app.channel_label(&raw), "Alice");
    }

    #[test]
    fn a_label_falls_back_to_the_peers_nickname_when_there_is_no_alias() {
        let mut app = app_with_dm("dm-whoever", PEER_ID);
        app.handle_net_event(NetEvent::Identity(
            "general".to_string(),
            IdentityAnnounce {
                sender: PEER_ID,
                nickname: "alice".to_string(),
            },
        ));

        assert_eq!(app.channel_label("dm-whoever"), "alice (07070707)");
    }

    #[test]
    fn a_dm_invite_recipients_tab_is_labeled_by_the_inviter() {
        // The recipient of a /msg invite gets the channel under the name
        // the *initiator* chose for it -- their own label for the
        // recipient. So "dm-Bob" arriving from Alice is named for Bob
        // himself; the label must say who the chat is actually with.
        let mut app = app();
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        app.handle_net_event(NetEvent::DmJoined {
            channel: "dm-Bob".to_string(),
            peer: PEER_ID,
        });

        assert_eq!(app.active().name, "dm-Bob");
        assert_eq!(app.channel_label("dm-Bob"), "Alice");
    }

    #[test]
    fn a_msg_dm_and_a_dm_chat_with_the_same_peer_share_a_label_but_not_a_heading() {
        let raw = crate::direct::channel_name(&PEER_ID);
        let mut app = app_with_dm("dm-Alice", PEER_ID);
        app.background_direct_channel(raw.clone(), PEER_ID);
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        assert_eq!(app.channel_label("dm-Alice"), "Alice");
        assert_eq!(app.channel_label(&raw), "Alice");
        assert_eq!(app.channel_heading("dm-Alice"), "@Alice");
        assert_eq!(app.channel_heading(&raw), "⚡Alice");
    }

    #[test]
    fn leave_accepts_a_dm_by_the_name_shown_on_screen() {
        let mut app = app_with_dm("dm-07070707", PEER_ID);
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/leave Alice");

        match action.expect("a label that resolves should leave") {
            InputAction::Leave(channel) => assert_eq!(channel, "dm-07070707"),
            _ => panic!("expected InputAction::Leave"),
        }
    }

    #[test]
    fn leave_still_accepts_a_channels_raw_name() {
        let mut app = app_with_dm("dm-07070707", PEER_ID);
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/leave dm-07070707");

        match action.expect("a raw name should still leave") {
            InputAction::Leave(channel) => assert_eq!(channel, "dm-07070707"),
            _ => panic!("expected InputAction::Leave"),
        }
    }

    #[test]
    fn leave_prefers_an_exact_channel_name_over_a_matching_label() {
        // "Alice" is both a group channel's own name and the DM's label.
        let mut app = AppState::new(
            SELF_ID,
            vec![
                "general".to_string(),
                "Alice".to_string(),
                "dm-07070707".to_string(),
            ],
            "general",
        );
        app.record_dm_channel(PEER_ID, "dm-07070707".to_string());
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        let action = submit(&mut app, "/leave Alice");

        match action.expect("an exact name should win") {
            InputAction::Leave(channel) => assert_eq!(channel, "Alice"),
            _ => panic!("expected InputAction::Leave"),
        }
    }

    #[test]
    fn leave_refuses_a_label_shared_by_several_channels() {
        let mut app = AppState::new(
            SELF_ID,
            vec![
                "general".to_string(),
                "dm-07070707".to_string(),
                "dm-08080808".to_string(),
            ],
            "general",
        );
        app.record_dm_channel(PEER_ID, "dm-07070707".to_string());
        app.record_dm_channel(PEER_ID_2, "dm-08080808".to_string());
        app.load_contacts(HashMap::from([
            (PEER_ID, "Alice".to_string()),
            (PEER_ID_2, "Alice".to_string()),
        ]));

        let action = submit(&mut app, "/leave Alice");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("more than one channel"), "got: {last}");
    }

    #[test]
    fn who_names_a_dm_by_its_heading() {
        let mut app = app_with_dm("dm-07070707", PEER_ID);
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        app.active = 1; // the DM tab

        let action = submit(&mut app, "/who");

        assert!(action.is_none());
        assert_eq!(app.viewing_peers.as_ref().unwrap().heading, "@Alice");
    }

    /// `general`, a `/msg` DM (`dm-Alice`), `project-x`, and a `/dm` chat
    /// (`p2p-08080808`) -- joined in that order, so the two kinds are
    /// interleaved -- with `general` active. The shape the navigation tests
    /// below all start from.
    fn app_with_interleaved_kinds() -> AppState {
        let mut app = AppState::new(
            SELF_ID,
            vec![
                "general".to_string(),
                "dm-Alice".to_string(),
                "project-x".to_string(),
            ],
            "general",
        );
        app.record_dm_channel(PEER_ID, "dm-Alice".to_string());
        app.background_direct_channel(crate::direct::channel_name(&PEER_ID_2), PEER_ID_2);
        app
    }

    #[test]
    fn each_channel_kind_has_its_own_glyph() {
        let glyphs = [
            ChannelKind::Group.glyph(),
            ChannelKind::GossipDm { peer: PEER_ID }.glyph(),
            ChannelKind::Direct { peer: PEER_ID }.glyph(),
        ];

        assert_eq!(glyphs, ["#", "@", "⚡"]);
    }

    #[test]
    fn a_channels_heading_is_its_label_behind_its_kind_glyph() {
        let mut app = app_with_interleaved_kinds();
        app.load_contacts(HashMap::from([
            (PEER_ID, "Alice".to_string()),
            (PEER_ID_2, "Bob".to_string()),
        ]));

        assert_eq!(app.channel_heading("general"), "#general");
        assert_eq!(app.channel_heading("dm-Alice"), "@Alice");
        assert_eq!(app.channel_heading("p2p-08080808"), "⚡Bob");
    }

    #[test]
    fn nav_sections_split_group_channels_from_conversations_in_join_order() {
        let app = app_with_interleaved_kinds();

        let sections = app.nav_sections();

        assert_eq!(sections.channels, vec![0, 2], "general and project-x");
        assert_eq!(sections.people, vec![1, 3], "the DM and the /dm chat");
    }

    #[test]
    fn nav_sections_of_only_group_channels_has_no_people() {
        let sections = multi_channel_app().nav_sections();

        assert_eq!(sections.channels, vec![0, 1]);
        assert!(sections.people.is_empty());
    }

    #[test]
    fn tab_walks_the_panel_top_to_bottom_and_wraps() {
        let mut app = app_with_interleaved_kinds();
        let mut visited = Vec::new();

        for _ in 0..4 {
            app.handle_key(key(KeyCode::Tab));
            visited.push(app.active().name.clone());
        }

        // Group channels first (general -> project-x), then conversations
        // (dm-Alice -> the /dm chat): the order the panel draws them in,
        // not the order they were joined in. Then back to the top.
        assert_eq!(
            visited,
            ["project-x", "dm-Alice", "p2p-08080808", "general"]
        );
    }

    #[test]
    fn backtab_walks_the_panel_bottom_to_top_and_wraps() {
        let mut app = app_with_interleaved_kinds();
        let mut visited = Vec::new();

        for _ in 0..4 {
            app.handle_key(key(KeyCode::BackTab));
            visited.push(app.active().name.clone());
        }

        assert_eq!(
            visited,
            ["p2p-08080808", "dm-Alice", "project-x", "general"]
        );
    }

    #[test]
    fn leave_accepts_a_channel_by_its_heading() {
        let mut app = app_with_interleaved_kinds();
        app.load_contacts(HashMap::from([
            (PEER_ID, "Alice".to_string()),
            (PEER_ID_2, "Bob".to_string()),
        ]));

        for (typed, expected) in [
            ("#project-x", "project-x"),
            ("@Alice", "dm-Alice"),
            ("⚡Bob", "p2p-08080808"),
        ] {
            match submit(&mut app, &format!("/leave {typed}")) {
                Some(InputAction::Leave(channel)) => assert_eq!(channel, expected, "{typed}"),
                _ => panic!("expected InputAction::Leave for {typed}"),
            }
        }
    }

    #[test]
    fn a_heading_tells_apart_a_dm_and_a_dm_chat_with_the_same_person() {
        let raw = crate::direct::channel_name(&PEER_ID);
        let mut app = app_with_dm("dm-Alice", PEER_ID);
        app.background_direct_channel(raw.clone(), PEER_ID);
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        // The bare label is shared, so on its own it can't say which one
        // is meant...
        assert!(submit(&mut app, "/leave Alice").is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("more than one channel"), "got: {last}");

        // ...but each one's heading can.
        match submit(&mut app, "/leave @Alice") {
            Some(InputAction::Leave(channel)) => assert_eq!(channel, "dm-Alice"),
            _ => panic!("expected InputAction::Leave for the DM"),
        }
        match submit(&mut app, "/leave ⚡Alice") {
            Some(InputAction::Leave(channel)) => assert_eq!(channel, raw),
            _ => panic!("expected InputAction::Leave for the /dm chat"),
        }
    }

    /// `app_with_interleaved_kinds` with its two peers named, so the labels
    /// are `general`, `Alice` (a `/msg` DM), `project-x` and `Bob` (a `/dm`
    /// chat): channel indices 0 to 3, but 0, 2, 1, 3 in the order the panel
    /// draws them.
    fn app_for_the_switcher() -> AppState {
        let mut app = app_with_interleaved_kinds();
        app.load_contacts(HashMap::from([
            (PEER_ID, "Alice".to_string()),
            (PEER_ID_2, "Bob".to_string()),
        ]));
        app
    }

    fn ctrl_p(app: &mut AppState) {
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));
    }

    /// Types `text` one key at a time without pressing `Enter` -- into
    /// whatever has the keyboard, the switcher's query once it's open.
    fn type_text(app: &mut AppState, text: &str) {
        for c in text.chars() {
            app.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn switcher_query(app: &AppState) -> &str {
        &app.switching.as_ref().expect("the switcher is open").query
    }

    #[test]
    fn ctrl_p_opens_the_switcher_listing_every_channel_in_panel_order() {
        let mut app = app_for_the_switcher();
        assert!(app.switching.is_none());
        assert!(
            app.switcher_matches().is_empty(),
            "nothing is listed while it is closed"
        );

        ctrl_p(&mut app);

        assert!(app.switching.is_some());
        assert_eq!(app.switcher_matches(), vec![0, 2, 1, 3]);
    }

    #[test]
    fn typing_narrows_the_switcher_to_labels_containing_the_text_ignoring_case() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);

        type_text(&mut app, "ECT"); // from the middle of "project-x"
        assert_eq!(app.switcher_matches(), vec![2]);

        type_text(&mut app, "z");
        assert!(app.switcher_matches().is_empty());
    }

    #[test]
    fn the_switcher_matches_the_label_not_the_stored_channel_name() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);

        // `dm-Alice` and `p2p-08080808` are the stored names; what is shown
        // for them is `Alice` and `Bob`.
        type_text(&mut app, "dm-");
        assert!(app.switcher_matches().is_empty());
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Backspace));
        }
        type_text(&mut app, "p2p");
        assert!(app.switcher_matches().is_empty());
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Backspace));
        }
        type_text(&mut app, "ali");
        assert_eq!(app.switcher_matches(), vec![1]);
    }

    #[test]
    fn backspace_widens_the_switcher_again() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);
        type_text(&mut app, "zz");
        assert!(app.switcher_matches().is_empty());

        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.switcher_matches(), vec![0, 2, 1, 3]);

        app.handle_key(key(KeyCode::Backspace)); // nothing left to delete
        assert_eq!(switcher_query(&app), "");
    }

    #[test]
    fn up_and_down_move_the_switcher_highlight_and_clamp_at_both_ends() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);
        assert_eq!(app.switcher_highlight(), Some(0));

        // Down walks the panel order: 0, 2, 1, 3 -- and stops at the end.
        let mut seen = Vec::new();
        for _ in 0..4 {
            app.handle_key(key(KeyCode::Down));
            seen.push(app.switcher_highlight());
        }
        assert_eq!(seen, [Some(2), Some(1), Some(3), Some(3)]);

        let mut seen = Vec::new();
        for _ in 0..4 {
            app.handle_key(key(KeyCode::Up));
            seen.push(app.switcher_highlight());
        }
        assert_eq!(seen, [Some(1), Some(2), Some(0), Some(0)]);
    }

    #[test]
    fn editing_the_query_puts_the_switcher_highlight_back_on_the_first_match() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.switcher_highlight(), Some(1));

        type_text(&mut app, "o"); // project-x and Bob
        assert_eq!(app.switcher_matches(), vec![2, 3]);
        assert_eq!(app.switcher_highlight(), Some(2));

        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.switcher_highlight(), Some(3));
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.switcher_highlight(), Some(0));
    }

    #[test]
    fn enter_in_the_switcher_switches_to_the_highlighted_channel_and_closes_it() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);
        type_text(&mut app, "bob");

        let action = app.handle_key(key(KeyCode::Enter));

        assert!(action.is_none());
        assert_eq!(app.active().name, "p2p-08080808");
        assert!(app.switching.is_none());
    }

    #[test]
    fn the_arrow_keys_pick_which_match_enter_switches_to() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);

        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.active().name, "dm-Alice");
    }

    #[test]
    fn switching_through_the_switcher_clears_unread_and_an_armed_reply() {
        let mut app = app_for_the_switcher();
        app.channels[3].unread_count = 1;
        app.channels[3].first_unread = Some(7);
        app.replying_to = Some(7);
        ctrl_p(&mut app);
        type_text(&mut app, "bob");

        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.active().unread_count, 0);
        assert_eq!(app.active().unread_anchor, Some(7));
        assert!(app.replying_to.is_none());
    }

    #[test]
    fn choosing_the_channel_you_are_already_on_just_closes_the_switcher() {
        let mut app = app_for_the_switcher();
        app.replying_to = Some(7);
        ctrl_p(&mut app);
        type_text(&mut app, "general");

        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.active, 0);
        assert!(app.switching.is_none());
        assert_eq!(
            app.replying_to,
            Some(7),
            "staying put must not discard an armed reply"
        );
    }

    #[test]
    fn enter_with_nothing_matching_keeps_the_switcher_open() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);
        type_text(&mut app, "zzz");

        app.handle_key(key(KeyCode::Enter));

        assert!(app.switching.is_some());
        assert_eq!(app.active, 0);
    }

    #[test]
    fn esc_closes_the_switcher_without_switching_or_quitting() {
        let mut app = app_for_the_switcher();
        app.replying_to = Some(7);
        ctrl_p(&mut app);
        app.handle_key(key(KeyCode::Down)); // highlight another channel

        app.handle_key(key(KeyCode::Esc));

        assert!(app.switching.is_none());
        assert_eq!(app.active, 0, "cancelling leaves the active channel alone");
        assert!(!app.should_quit);
        assert_eq!(app.replying_to, Some(7));
    }

    #[test]
    fn ctrl_c_in_the_switcher_still_quits() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(app.should_quit);
    }

    #[test]
    fn keys_typed_into_the_switcher_never_reach_the_input_draft() {
        let mut app = app_for_the_switcher();
        type_text(&mut app, "draft");
        ctrl_p(&mut app);

        type_text(&mut app, "bob");
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Left));
        app.handle_key(key(KeyCode::Esc));

        assert_eq!(app.active().input, "draft");
        assert_eq!(app.active().cursor, 5);
    }

    #[test]
    fn control_chords_do_not_type_into_the_switcher_query() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        ctrl_p(&mut app); // a second Ctrl+P is just another chord to ignore

        assert_eq!(switcher_query(&app), "");
        assert!(app.switching.is_some());
    }

    #[test]
    fn a_paste_filters_the_switcher_instead_of_the_draft() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);

        app.paste_text("bo\nb\r\n");

        assert_eq!(switcher_query(&app), "bob");
        assert_eq!(app.active().input, "");
        assert_eq!(app.switcher_matches(), vec![3]);
    }

    #[test]
    fn the_switcher_highlight_stays_in_range_when_the_matches_shrink_underneath() {
        let mut app = app_for_the_switcher();
        ctrl_p(&mut app);
        for _ in 0..3 {
            app.handle_key(key(KeyCode::Down));
        }
        assert_eq!(app.switcher_highlight(), Some(3)); // Bob, the last match

        app.remove_channel("p2p-08080808"); // gone while the overlay is open

        assert_eq!(app.switcher_matches(), vec![0, 2, 1]);
        assert_eq!(
            app.switcher_highlight(),
            Some(1),
            "clamped to the new last match"
        );
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.active().name, "dm-Alice");
    }

    #[test]
    fn the_switcher_follows_labels_that_change_while_it_is_open() {
        let mut app = app_with_interleaved_kinds(); // peers unnamed: hex labels
        ctrl_p(&mut app);
        type_text(&mut app, "ali");
        assert!(app.switcher_matches().is_empty());

        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        assert_eq!(app.switcher_matches(), vec![1]);
    }

    #[test]
    fn ctrl_p_is_ignored_while_another_mode_has_the_keyboard() {
        let mut app = app_for_the_switcher();
        app.picking_reply = Some(1);
        ctrl_p(&mut app);
        assert!(app.switching.is_none());
        assert_eq!(app.picking_reply, Some(1));

        app.picking_reply = None;
        app.pending_redeem = Some("ticket".to_string());
        ctrl_p(&mut app);
        assert!(app.switching.is_none());
    }

    // -- the files overlay ----------------------------------------------------
    // These reuse the switcher fixture (`app_for_the_switcher`) where they
    // need every kind of conversation: `general` (active), a `/msg` DM shown
    // as `Alice`, `project-x`, and a `/dm` chat shown as `Bob`.

    /// Opens the files overlay with `/files`, returning what that asked of
    /// the caller.
    fn open_files(app: &mut AppState) -> Option<InputAction> {
        submit(app, "/files")
    }

    fn ctrl_o(app: &mut AppState) -> Option<InputAction> {
        app.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL))
    }

    fn files_query(app: &AppState) -> &str {
        &app.viewing_files
            .as_ref()
            .expect("the files overlay is open")
            .query
    }

    fn files_notice(app: &AppState) -> Option<FilesNotice> {
        app.viewing_files
            .as_ref()
            .expect("the files overlay is open")
            .notice
    }

    /// The overlay open on `general`, whose one file was announced as
    /// `announced` and -- if the disk has said so -- is saved at `saved_at`.
    fn app_viewing_file(announced: &str, saved_at: Option<&str>) -> AppState {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, announced, b"content"),
        );
        open_files(&mut app);
        if let Some(path) = saved_at {
            mark_saved(&mut app, "general", hash_of(b"content"), path);
        }
        app
    }

    /// `app_viewing_file` on `report.pdf`, `saved` or not.
    fn app_viewing_report(saved: bool) -> AppState {
        app_viewing_file("report.pdf", saved.then_some("/files/general/report.pdf"))
    }

    /// The names of the files the open overlay lists, top to bottom.
    fn listed(app: &AppState) -> Vec<String> {
        app.files_matches()
            .iter()
            .map(|entry| entry.filename().to_string())
            .collect()
    }

    /// The hash `file_message` gives an attachment with `content`.
    fn hash_of(content: &[u8]) -> Hash {
        Hash::new(content)
    }

    /// Every field of a `SaveFile` action -- channel, hash, filename,
    /// providers, folder -- panicking on anything else, to compare what the
    /// overlay asks for with what `/save` asks for.
    fn save_fields(action: Option<InputAction>) -> (String, Hash, String, Vec<[u8; 32]>, PathBuf) {
        match action.expect("a save returns an action") {
            InputAction::SaveFile(request) => (
                request.channel,
                request.hash,
                request.filename,
                request.providers,
                request.dir,
            ),
            _ => panic!("expected InputAction::SaveFile"),
        }
    }

    /// Tells the open overlay that `channel`'s copy of `hash` is at `path`,
    /// as the caller does when it answers `InputAction::CheckSavedFiles`.
    fn mark_saved(app: &mut AppState, channel: &str, hash: Hash, path: &str) {
        app.apply_saved_files(channel, HashMap::from([(hash, PathBuf::from(path))]));
    }

    fn apply_local_files(app: &mut AppState, files: Vec<LocalFile>) {
        let overlay = app.viewing_files.as_ref().expect("files overlay is open");
        app.apply_files_listing(FilesListing {
            request: overlay.request,
            channel: overlay.channel.clone(),
            files: Ok(files),
        });
    }

    fn local_file(relative: &str, modified_unix_ms: u64) -> LocalFile {
        LocalFile {
            path: PathBuf::from("/files/general").join(relative),
            relative: relative.to_string(),
            size: 7,
            modified_unix_ms,
        }
    }

    /// `general` with four files -- two from Alice, one from Bob, one of our
    /// own -- listed newest first as `mine.zip`, `notes.txt`, `holiday.png`,
    /// `Quarterly-Report.PDF`.
    fn app_with_a_few_files() -> AppState {
        let mut app = app();
        app.load_contacts(HashMap::from([
            (PEER_ID, "Alice".to_string()),
            (PEER_ID_2, "Bob".to_string()),
        ]));
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "Quarterly-Report.PDF", b"q"),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID_2, "holiday.png", b"h"),
        );
        receive(
            &mut app,
            "general",
            file_message(3, PEER_ID, "notes.txt", b"n"),
        );
        app.record_sent_message("general", file_message(4, SELF_ID, "mine.zip", b"m"));
        app
    }

    #[test]
    fn slash_files_opens_the_overlay_on_the_active_channel_and_reads_its_folder() {
        let mut app = multi_channel_app(); // "random" is active

        let action = open_files(&mut app);

        match action.expect("/files returns an action") {
            InputAction::ReadFiles { channel, .. } => assert_eq!(channel, "random"),
            _ => panic!("expected InputAction::ReadFiles"),
        }
        let overlay = app.viewing_files.as_ref().expect("the overlay is open");
        assert_eq!(overlay.channel, "random");
        assert_eq!(overlay.query, "");
    }

    #[test]
    fn synced_files_join_shared_files_and_match_nested_relative_paths() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        open_files(&mut app);
        apply_local_files(
            &mut app,
            vec![local_file("docs/project-plan.md", now_unix_ms())],
        );

        assert_eq!(listed(&app), ["docs/project-plan.md", "report.pdf"]);
        type_text(&mut app, "project");
        assert_eq!(listed(&app), ["docs/project-plan.md"]);
    }

    #[test]
    fn a_local_scan_deduplicates_a_saved_shared_copy() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        open_files(&mut app);
        mark_saved(
            &mut app,
            "general",
            hash_of(b"report"),
            "/files/general/report.pdf",
        );
        apply_local_files(&mut app, vec![local_file("report.pdf", now_unix_ms())]);

        assert_eq!(app.overlay_files().len(), 1);
        assert!(matches!(
            app.overlay_files().as_slice(),
            [FilesEntry::Shared(_)]
        ));
    }

    #[test]
    fn a_late_files_listing_does_not_update_a_newer_overlay() {
        let mut app = app();
        open_files(&mut app);
        let request = app.viewing_files.as_ref().unwrap().request;
        let channel = app.viewing_files.as_ref().unwrap().channel.clone();
        app.handle_key(key(KeyCode::Esc));
        open_files(&mut app);

        app.apply_files_listing(FilesListing {
            request,
            channel,
            files: Ok(vec![local_file("stale.md", now_unix_ms())]),
        });

        assert!(app.overlay_files().is_empty());
    }

    #[test]
    fn local_files_offer_open_reveal_and_edit_actions() {
        let mut app = app();
        open_files(&mut app);
        apply_local_files(&mut app, vec![local_file("notes.txt", now_unix_ms())]);

        assert!(matches!(
            ctrl_o(&mut app),
            Some(InputAction::OpenFile(path)) if path == PathBuf::from("/files/general/notes.txt")
        ));

        open_files(&mut app);
        apply_local_files(&mut app, vec![local_file("notes.txt", now_unix_ms())]);
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(InputAction::RevealFile(path)) if path == PathBuf::from("/files/general/notes.txt")
        ));

        open_files(&mut app);
        apply_local_files(&mut app, vec![local_file("notes.txt", now_unix_ms())]);
        assert!(matches!(
            app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)),
            Some(InputAction::EditFile(path)) if path == PathBuf::from("/files/general/notes.txt")
        ));
    }

    #[test]
    fn the_overlay_lists_the_files_of_the_channel_newest_first_and_nothing_else() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "old.pdf", b"old"),
        );
        receive(
            &mut app,
            "general",
            compose_message(PEER_ID, "chatter", None),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID_2, "mid.pdf", b"mid"),
        );
        app.push_system("a notice");
        receive(
            &mut app,
            "general",
            file_message(3, PEER_ID, "new.pdf", b"new"),
        );

        open_files(&mut app);

        assert_eq!(listed(&app), ["new.pdf", "mid.pdf", "old.pdf"]);
    }

    #[test]
    fn a_row_carries_the_name_size_sender_and_date_of_its_share() {
        let mut app = app();
        let sent = ChatMessage {
            ts_unix_ms: 1_760_000_000_000,
            ..file_message(1, PEER_ID, "report.pdf", b"six by")
        };
        receive(&mut app, "general", sent);
        open_files(&mut app);

        let files = app.files_matches();

        assert_eq!(files.len(), 1);
        let FilesEntry::Shared(shared) = &files[0] else {
            panic!("expected shared attachment");
        };
        assert_eq!(shared.file.filename, "report.pdf");
        assert_eq!(shared.file.size, 6);
        assert_eq!(shared.file.hash, hash_of(b"six by"));
        assert_eq!(shared.sender, PEER_ID);
        assert_eq!(shared.ts_unix_ms, 1_760_000_000_000);
    }

    #[test]
    fn loaded_backfilled_and_live_files_are_all_listed() {
        let mut app = app();
        app.load_history(
            "general",
            vec![file_message(10, PEER_ID, "loaded.pdf", b"loaded")],
        );
        app.merge_history(
            "general",
            vec![file_message(5, PEER_ID_2, "backfilled.pdf", b"backfilled")],
        );
        receive(
            &mut app,
            "general",
            file_message(20, PEER_ID, "live.pdf", b"live"),
        );

        open_files(&mut app);

        assert_eq!(listed(&app), ["live.pdf", "loaded.pdf", "backfilled.pdf"]);
    }

    #[test]
    fn a_file_shared_twice_is_listed_once_per_share_with_its_own_sender() {
        let mut app = app();
        let content = b"same bytes";
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", content),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID_2, "report-final.pdf", content),
        );
        open_files(&mut app);

        let files = app.files_matches();

        let rows: Vec<(&str, [u8; 32])> = files
            .iter()
            .map(|entry| match entry {
                FilesEntry::Shared(shared) => (shared.file.filename.as_str(), shared.sender),
                FilesEntry::Local(_) => panic!("fixture has no local files"),
            })
            .collect();
        assert_eq!(
            rows,
            [("report-final.pdf", PEER_ID_2), ("report.pdf", PEER_ID)]
        );
    }

    #[test]
    fn our_own_files_are_listed_too() {
        let mut app = app();
        app.record_sent_message("general", file_message(1, SELF_ID, "mine.pdf", b"mine"));

        open_files(&mut app);

        assert_eq!(listed(&app), ["mine.pdf"]);
        assert!(matches!(
            app.files_matches()[0],
            FilesEntry::Shared(SharedFile {
                sender: SELF_ID,
                ..
            })
        ));
    }

    #[test]
    fn only_the_files_of_the_channel_it_was_opened_on_are_listed() {
        let mut app = multi_channel_app(); // "random" is active
        receive(
            &mut app,
            "random",
            file_message(1, PEER_ID, "here.pdf", b"here"),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID, "elsewhere.pdf", b"elsewhere"),
        );

        open_files(&mut app);

        assert_eq!(listed(&app), ["here.pdf"]);
    }

    #[test]
    fn the_overlay_works_in_group_channels_dms_and_dm_chats() {
        let mut app = app_for_the_switcher();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "group.pdf", b"group"),
        );
        receive(
            &mut app,
            "dm-Alice",
            file_message(2, PEER_ID, "dm.pdf", b"dm"),
        );
        receive(
            &mut app,
            "p2p-08080808",
            file_message(3, PEER_ID_2, "chat.pdf", b"chat"),
        );

        for (index, name, expected) in [
            (0, "general", "group.pdf"),
            (1, "dm-Alice", "dm.pdf"),
            (3, "p2p-08080808", "chat.pdf"),
        ] {
            app.active = index;
            match open_files(&mut app).expect("/files returns an action") {
                InputAction::ReadFiles { channel, .. } => assert_eq!(channel, name),
                _ => panic!("expected InputAction::ReadFiles"),
            }
            assert_eq!(listed(&app), [expected], "{name}");
            app.handle_key(key(KeyCode::Esc));
        }
    }

    #[test]
    fn an_empty_channel_opens_an_overlay_with_nothing_in_it() {
        let mut app = app();

        open_files(&mut app);

        assert!(app.viewing_files.is_some());
        assert!(app.files_matches().is_empty());
        assert_eq!(app.files_highlight(), None);
        // Nothing to save or open, so those keys leave it as it is.
        assert!(app.handle_key(key(KeyCode::Enter)).is_none());
        assert!(ctrl_o(&mut app).is_none());
        assert!(app.viewing_files.is_some());
        assert_eq!(files_notice(&app), None, "no file, nothing to say about it");
    }

    #[test]
    fn typing_narrows_the_list_to_names_containing_the_text_ignoring_case() {
        let mut app = app_with_a_few_files();
        open_files(&mut app);
        assert_eq!(
            listed(&app),
            [
                "mine.zip",
                "notes.txt",
                "holiday.png",
                "Quarterly-Report.PDF"
            ]
        );

        type_text(&mut app, "REPORT");
        assert_eq!(listed(&app), ["Quarterly-Report.PDF"]);

        type_text(&mut app, "x");
        assert!(listed(&app).is_empty());
    }

    #[test]
    fn typing_also_narrows_by_the_sender_as_shown() {
        let mut app = app_with_a_few_files();
        open_files(&mut app);

        type_text(&mut app, "ALI");
        assert_eq!(
            listed(&app),
            ["notes.txt", "Quarterly-Report.PDF"],
            "Alice's files"
        );

        for _ in 0..3 {
            app.handle_key(key(KeyCode::Backspace));
        }
        type_text(&mut app, "bob");
        assert_eq!(listed(&app), ["holiday.png"]);

        for _ in 0..3 {
            app.handle_key(key(KeyCode::Backspace));
        }
        type_text(&mut app, "you");
        assert_eq!(listed(&app), ["mine.zip"], "our own are from you");
    }

    #[test]
    fn a_sender_is_matched_by_what_is_shown_not_by_the_raw_id() {
        let mut app = app();
        receive(&mut app, "general", file_message(1, PEER_ID, "a.pdf", b"a"));
        open_files(&mut app);

        // Unnamed, a peer is shown by the first bytes of its id.
        type_text(&mut app, "0707");
        assert_eq!(listed(&app), ["a.pdf"]);

        // Named, it is shown by its name -- which the hex no longer matches.
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));
        assert!(listed(&app).is_empty());
    }

    #[test]
    fn up_and_down_move_the_highlight_and_clamp_at_both_ends() {
        let mut app = app_with_a_few_files();
        open_files(&mut app);
        assert_eq!(app.files_highlight(), Some(0));

        let mut seen = Vec::new();
        for _ in 0..5 {
            app.handle_key(key(KeyCode::Down));
            seen.push(app.files_highlight());
        }
        assert_eq!(seen, [Some(1), Some(2), Some(3), Some(3), Some(3)]);

        let mut seen = Vec::new();
        for _ in 0..5 {
            app.handle_key(key(KeyCode::Up));
            seen.push(app.files_highlight());
        }
        assert_eq!(seen, [Some(2), Some(1), Some(0), Some(0), Some(0)]);
    }

    #[test]
    fn editing_the_query_puts_the_highlight_back_on_the_first_match() {
        let mut app = app_with_a_few_files();
        open_files(&mut app);
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.files_highlight(), Some(2));

        // From the name `holiday.png` and `Quarterly-Report.PDF`, and from
        // the sender of `mine.zip`, `you`.
        type_text(&mut app, "y");
        assert_eq!(
            listed(&app),
            ["mine.zip", "holiday.png", "Quarterly-Report.PDF"]
        );
        assert_eq!(app.files_highlight(), Some(0));

        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.files_highlight(), Some(2));
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.files_highlight(), Some(0));
    }

    #[test]
    fn the_highlight_stays_in_range_when_the_matches_shrink_underneath() {
        let mut app = app();
        receive(&mut app, "general", file_message(1, PEER_ID, "a.pdf", b"a"));
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID_2, "b.pdf", b"b"),
        );
        receive(
            &mut app,
            "general",
            file_message(3, PEER_ID_2, "c.pdf", b"c"),
        );
        open_files(&mut app);
        type_text(&mut app, "0"); // in the hex of both unnamed peers
        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::Down));
        assert_eq!(listed(&app), ["c.pdf", "b.pdf", "a.pdf"]);
        assert_eq!(app.files_highlight(), Some(2));

        // Alice is no longer shown as hex, so `a.pdf` stops matching.
        app.load_contacts(HashMap::from([(PEER_ID, "Alice".to_string())]));

        assert_eq!(listed(&app), ["c.pdf", "b.pdf"]);
        assert_eq!(
            app.files_highlight(),
            Some(1),
            "clamped to the new last match"
        );
    }

    #[test]
    fn esc_closes_the_overlay_without_quitting_or_acting() {
        let mut app = app_with_a_few_files();
        open_files(&mut app);
        let before = app.active().messages.len();

        let action = app.handle_key(key(KeyCode::Esc));

        assert!(action.is_none());
        assert!(app.viewing_files.is_none());
        assert!(!app.should_quit);
        assert_eq!(app.active().messages.len(), before);
    }

    #[test]
    fn ctrl_c_in_the_overlay_still_quits() {
        let mut app = app_with_a_few_files();
        open_files(&mut app);

        app.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));

        assert!(app.should_quit);
    }

    #[test]
    fn keys_typed_into_the_overlay_never_reach_the_input_draft() {
        let mut app = multi_channel_app();
        open_files(&mut app);

        type_text(&mut app, "report");
        app.handle_key(key(KeyCode::Backspace));
        app.handle_key(key(KeyCode::Left));
        app.handle_key(key(KeyCode::Tab));

        assert_eq!(app.active().input, "");
        assert_eq!(app.active().cursor, 0);
        assert_eq!(app.active, 1, "Tab did not switch channels either");
        assert_eq!(files_query(&app), "repor");
    }

    #[test]
    fn control_chords_do_not_type_into_the_query_or_open_other_overlays() {
        let mut app = app_with_a_few_files();
        open_files(&mut app);

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        ctrl_p(&mut app);

        assert_eq!(files_query(&app), "");
        assert!(app.viewing_files.is_some());
        assert!(app.switching.is_none());
    }

    #[test]
    fn a_paste_filters_the_overlay_instead_of_the_draft() {
        let mut app = app_with_a_few_files();
        open_files(&mut app);

        app.paste_text("rep\nort\r\n");

        assert_eq!(files_query(&app), "report");
        assert_eq!(app.active().input, "");
        assert_eq!(listed(&app), ["Quarterly-Report.PDF"]);
    }

    #[test]
    fn nothing_is_composing_while_the_overlay_is_open() {
        let mut app = app();
        assert!(app.is_composing());

        open_files(&mut app);
        assert!(!app.is_composing());

        app.handle_key(key(KeyCode::Esc));
        assert!(app.is_composing());
    }

    #[test]
    fn enter_saves_the_highlighted_file_exactly_as_slash_save_would() {
        let mut app = app();
        app.load_files_root(PathBuf::from("/files"));
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "old.pdf", b"old"),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID_2, "new.pdf", b"new"),
        );
        peers_join(&mut app, "general", &[PEER_ID_3]);
        // A bare `/save` asks for the newest file, which is the one the
        // overlay starts on.
        let by_command = save_fields(submit(&mut app, "/save"));
        open_files(&mut app);

        let by_overlay = save_fields(app.handle_key(key(KeyCode::Enter)));

        assert_eq!(by_overlay, by_command);
        assert_eq!(by_overlay.0, "general");
        assert_eq!(by_overlay.2, "new.pdf");
        assert_eq!(by_overlay.3, vec![PEER_ID_2, PEER_ID_3, PEER_ID]);
        assert_eq!(by_overlay.4, PathBuf::from("/files/general"));
    }

    #[test]
    fn enter_closes_the_overlay_and_leaves_the_saving_notice_in_the_transcript() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        open_files(&mut app);

        app.handle_key(key(KeyCode::Enter));

        assert!(app.viewing_files.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "saving report.pdf...");
    }

    #[test]
    fn enter_saves_the_file_that_is_highlighted_not_just_the_newest() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "old.pdf", b"old"),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID, "new.pdf", b"new"),
        );
        open_files(&mut app);
        app.handle_key(key(KeyCode::Down));

        let (_, hash, filename, ..) = save_fields(app.handle_key(key(KeyCode::Enter)));

        assert_eq!((hash, filename.as_str()), (hash_of(b"old"), "old.pdf"));
    }

    #[test]
    fn enter_saves_one_of_our_own_files_from_our_own_store() {
        let mut app = app();
        app.record_sent_message("general", file_message(1, SELF_ID, "mine.pdf", b"mine"));
        peers_join(&mut app, "general", &[PEER_ID]);
        open_files(&mut app);

        let (_, _, filename, providers, _) = save_fields(app.handle_key(key(KeyCode::Enter)));

        assert_eq!(filename, "mine.pdf");
        assert_eq!(
            providers,
            vec![SELF_ID, PEER_ID],
            "this device is the sender, and holds the file"
        );
    }

    #[test]
    fn enter_saves_into_the_folder_of_each_kind_of_conversation() {
        let mut app = app_for_the_switcher();
        app.load_files_root(PathBuf::from("/files"));
        receive(
            &mut app,
            "dm-Alice",
            file_message(1, PEER_ID, "dm.pdf", b"dm"),
        );
        receive(
            &mut app,
            "p2p-08080808",
            file_message(2, PEER_ID_2, "chat.pdf", b"chat"),
        );

        let mut dirs = Vec::new();
        for index in [1, 3] {
            app.active = index;
            open_files(&mut app);
            dirs.push(save_fields(app.handle_key(key(KeyCode::Enter))).4);
        }

        assert_eq!(
            dirs,
            [
                channel_dir(Path::new("/files"), "dm-Alice"),
                PathBuf::from("/files/p2p-08080808")
            ]
        );
    }

    #[test]
    fn the_overlay_stays_on_the_channel_it_was_opened_on_when_focus_moves() {
        let mut app = multi_channel_app(); // "random" is active
        app.load_files_root(PathBuf::from("/files"));
        receive(
            &mut app,
            "random",
            file_message(1, PEER_ID, "here.pdf", b"here"),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID, "there.pdf", b"there"),
        );
        open_files(&mut app);

        // A join finishing elsewhere takes focus.
        app.handle_net_event(NetEvent::Joined("general".to_string()));
        assert_eq!(app.active().name, "general");

        assert_eq!(listed(&app), ["here.pdf"], "still #random's list");
        let (channel, _, filename, _, dir) = save_fields(app.handle_key(key(KeyCode::Enter)));
        assert_eq!(
            (channel.as_str(), filename.as_str()),
            ("random", "here.pdf")
        );
        assert_eq!(dir, PathBuf::from("/files/random"));
    }

    #[test]
    fn enter_on_a_file_already_saved_does_nothing_so_it_is_never_saved_twice() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        open_files(&mut app);
        mark_saved(
            &mut app,
            "general",
            hash_of(b"report"),
            "/files/general/report.pdf",
        );
        let before = app.active().messages.len();

        let action = app.handle_key(key(KeyCode::Enter));

        assert!(action.is_none());
        assert!(app.viewing_files.is_some(), "still open");
        assert_eq!(
            app.active().messages.len(),
            before,
            "no `saving` notice either"
        );
    }

    #[test]
    fn enter_on_a_file_already_saved_says_so_and_what_to_press_instead_works() {
        let mut app = app_viewing_report(true);
        assert_eq!(files_notice(&app), None, "nothing to say before a key");

        app.handle_key(key(KeyCode::Enter));

        assert_eq!(files_notice(&app), Some(FilesNotice::AlreadySaved));
        match ctrl_o(&mut app).expect("the notice points at Ctrl+O, which opens it") {
            InputAction::OpenFile(path) => {
                assert_eq!(path, PathBuf::from("/files/general/report.pdf"));
            }
            _ => panic!("expected InputAction::OpenFile"),
        }
    }

    #[test]
    fn ctrl_o_on_a_file_that_is_not_saved_says_so_and_what_to_press_instead_works() {
        let mut app = app_viewing_report(false);

        assert!(ctrl_o(&mut app).is_none());

        assert_eq!(files_notice(&app), Some(FilesNotice::NotLocal));
        let (_, _, filename, ..) = save_fields(app.handle_key(key(KeyCode::Enter)));
        assert_eq!(filename, "report.pdf", "the notice points at Enter");
    }

    #[test]
    fn a_notice_is_gone_at_the_next_key_whatever_it_is() {
        let keys = [
            key(KeyCode::Up),
            key(KeyCode::Down),
            key(KeyCode::Char('x')),
            key(KeyCode::Backspace),
            key(KeyCode::Left), // not a key the overlay uses
            key(KeyCode::Tab),  // nor this
            KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL), // nor a chord
        ];
        for pressed in keys {
            let mut app = app_viewing_report(true);
            app.handle_key(key(KeyCode::Enter));
            assert_eq!(files_notice(&app), Some(FilesNotice::AlreadySaved));

            app.handle_key(pressed);

            assert_eq!(files_notice(&app), None, "after {pressed:?}");
        }
    }

    #[test]
    fn pressing_the_same_key_again_says_it_again() {
        let mut app = app_viewing_report(true);

        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(files_notice(&app), Some(FilesNotice::AlreadySaved));
    }

    #[test]
    fn a_paste_drops_the_notice_too() {
        let mut app = app_viewing_report(true);
        app.handle_key(key(KeyCode::Enter));

        app.paste_text("rep");

        assert_eq!(files_notice(&app), None);
    }

    #[test]
    fn a_notice_does_not_outlive_its_overlay() {
        let mut app = app_viewing_report(true);
        app.handle_key(key(KeyCode::Enter));
        app.handle_key(key(KeyCode::Esc));

        open_files(&mut app);

        assert_eq!(files_notice(&app), None);
    }

    #[test]
    fn what_a_file_is_offered_follows_whether_it_is_saved_and_whether_opening_it_would_run_it() {
        let unsaved_program = app_viewing_file("setup.exe", None);
        assert_eq!(
            unsaved_program.file_offer(&unsaved_program.files_matches()[0]),
            FileOffer::Save,
            "saving a program is as safe as saving anything"
        );

        let saved_document = app_viewing_file("report.pdf", Some("/files/general/report.pdf"));
        assert_eq!(
            saved_document.file_offer(&saved_document.files_matches()[0]),
            FileOffer::Open(PathBuf::from("/files/general/report.pdf"))
        );

        let saved_program = app_viewing_file("setup.exe", Some("/files/general/setup.exe"));
        assert_eq!(
            saved_program.file_offer(&saved_program.files_matches()[0]),
            FileOffer::SavedOnly
        );
    }

    #[test]
    fn ctrl_o_on_a_saved_program_is_refused_and_nothing_is_opened() {
        let mut app = app_viewing_file("setup.exe", Some("/files/general/setup.exe"));
        let before = app.active().messages.len();

        let action = ctrl_o(&mut app);

        assert!(action.is_none(), "nothing for the caller to open");
        assert!(app.viewing_files.is_some(), "still open");
        assert_eq!(files_notice(&app), Some(FilesNotice::SavedNotOpened));
        assert_eq!(
            app.active().messages.len(),
            before,
            "no `opening` line either"
        );
    }

    #[test]
    fn enter_on_a_saved_program_says_it_is_saved_and_is_not_opened_from_here() {
        let mut app = app_viewing_file("setup.exe", Some("/files/general/setup.exe"));
        let before = app.active().messages.len();

        let action = app.handle_key(key(KeyCode::Enter));

        assert!(action.is_none(), "not saved a second time");
        assert_eq!(files_notice(&app), Some(FilesNotice::SavedNotOpened));
        assert_eq!(app.active().messages.len(), before);
    }

    #[test]
    fn a_program_that_is_not_saved_yet_is_saved_like_any_file() {
        let mut app = app_viewing_file("setup.exe", None);

        let (_, _, filename, ..) = save_fields(app.handle_key(key(KeyCode::Enter)));

        assert_eq!(filename, "setup.exe");
    }

    #[test]
    fn the_saved_copys_name_decides_not_the_one_the_file_was_announced_under() {
        // `files::sanitize_filename` keeps only the last component of a name,
        // so a peer's "setup.exe/" is saved as `setup.exe` -- which is what
        // the OS would run.
        let mut app = app_viewing_file("setup.exe/", Some("/files/general/setup.exe"));

        assert!(ctrl_o(&mut app).is_none());

        assert_eq!(files_notice(&app), Some(FilesNotice::SavedNotOpened));
    }

    #[test]
    fn ctrl_o_opens_the_saved_copy_of_the_highlighted_file_and_closes_the_overlay() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        open_files(&mut app);
        mark_saved(
            &mut app,
            "general",
            hash_of(b"report"),
            "/files/general/report (1).pdf",
        );

        let action = ctrl_o(&mut app);

        match action.expect("Ctrl+O on a saved file returns an action") {
            InputAction::OpenFile(path) => {
                assert_eq!(path, PathBuf::from("/files/general/report (1).pdf"));
            }
            _ => panic!("expected InputAction::OpenFile"),
        }
        assert!(app.viewing_files.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "opening report.pdf...");
    }

    #[test]
    fn ctrl_o_on_a_file_that_is_not_saved_does_nothing() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        open_files(&mut app);
        let before = app.active().messages.len();

        let action = ctrl_o(&mut app);

        assert!(action.is_none());
        assert!(app.viewing_files.is_some(), "still open");
        assert_eq!(app.active().messages.len(), before);
    }

    #[test]
    fn ctrl_o_opens_whichever_file_is_highlighted() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "old.pdf", b"old"),
        );
        receive(
            &mut app,
            "general",
            file_message(2, PEER_ID, "new.pdf", b"new"),
        );
        open_files(&mut app);
        // Only the older one is saved.
        mark_saved(
            &mut app,
            "general",
            hash_of(b"old"),
            "/files/general/old.pdf",
        );

        assert!(
            ctrl_o(&mut app).is_none(),
            "new.pdf is highlighted, unsaved"
        );
        app.handle_key(key(KeyCode::Down));

        match ctrl_o(&mut app).expect("old.pdf is saved") {
            InputAction::OpenFile(path) => {
                assert_eq!(path, PathBuf::from("/files/general/old.pdf"));
            }
            _ => panic!("expected InputAction::OpenFile"),
        }
    }

    #[test]
    fn what_the_disk_says_only_reaches_the_overlay_it_is_about() {
        let mut app = multi_channel_app(); // "random" is active
        receive(
            &mut app,
            "random",
            file_message(1, PEER_ID, "here.pdf", b"here"),
        );
        open_files(&mut app);

        // An answer about #general, which the overlay isn't on.
        mark_saved(
            &mut app,
            "general",
            hash_of(b"here"),
            "/files/general/here.pdf",
        );
        assert_eq!(app.saved_copy(hash_of(b"here")), None);

        mark_saved(
            &mut app,
            "random",
            hash_of(b"here"),
            "/files/random/here.pdf",
        );
        assert_eq!(
            app.saved_copy(hash_of(b"here")),
            Some(Path::new("/files/random/here.pdf"))
        );
    }

    #[test]
    fn an_answer_after_the_overlay_closed_is_ignored_and_each_opening_starts_afresh() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        open_files(&mut app);
        mark_saved(
            &mut app,
            "general",
            hash_of(b"report"),
            "/files/general/report.pdf",
        );
        app.handle_key(key(KeyCode::Esc));

        // A late answer, with no overlay to give it to.
        mark_saved(
            &mut app,
            "general",
            hash_of(b"report"),
            "/files/general/report.pdf",
        );
        assert!(app.viewing_files.is_none());

        // Nothing is known about the disk until the caller has looked again.
        open_files(&mut app);
        assert_eq!(app.saved_copy(hash_of(b"report")), None);
    }

    #[test]
    fn a_save_that_finishes_while_the_overlay_is_open_marks_the_file_saved_at_once() {
        let mut app = app();
        receive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        open_files(&mut app);
        assert_eq!(app.saved_copy(hash_of(b"report")), None);

        app.handle_net_event(NetEvent::FileSaved {
            channel: "general".to_string(),
            hash: hash_of(b"report"),
            filename: "report.pdf".to_string(),
            path: PathBuf::from("/files/general/report.pdf"),
            mode: SaveMode::Manual,
        });

        assert_eq!(
            app.saved_copy(hash_of(b"report")),
            Some(Path::new("/files/general/report.pdf"))
        );
    }

    #[test]
    fn a_save_for_another_channel_does_not_mark_this_overlays_files() {
        let mut app = multi_channel_app(); // "random" is active
        receive(
            &mut app,
            "random",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );
        open_files(&mut app);

        app.handle_net_event(NetEvent::FileSaved {
            channel: "general".to_string(),
            hash: hash_of(b"report"),
            filename: "report.pdf".to_string(),
            path: PathBuf::from("/files/general/report.pdf"),
            mode: SaveMode::Manual,
        });

        assert_eq!(
            app.saved_copy(hash_of(b"report")),
            None,
            "a copy in #general's folder is not one in #random's"
        );
    }

    // The `/rename` tests below reuse the switcher fixture
    // (`app_for_the_switcher`): `general` (active), a `/msg` DM shown as
    // `Alice`, `project-x`, and a `/dm` chat shown as `Bob` -- channel
    // indices 0 to 3.

    #[test]
    fn rename_gives_the_active_group_channel_a_local_name() {
        let mut app = app_for_the_switcher();
        app.active = 2; // project-x

        let action = submit(&mut app, "/rename proj");

        match action.expect("a successful /rename returns an action") {
            InputAction::Rename { channel, name } => {
                assert_eq!(channel, "project-x");
                assert_eq!(name, Some("proj".to_string()));
            }
            _ => panic!("expected InputAction::Rename"),
        }
        assert_eq!(app.channel_label("project-x"), "proj");
        assert_eq!(app.channel_heading("project-x"), "#proj");
        assert_eq!(
            app.channels[2].name, "project-x",
            "the stored name is the channel's identity and never changes"
        );
        assert_eq!(
            app.channel_label("general"),
            "general",
            "other channels are untouched"
        );
    }

    #[test]
    fn a_renamed_channel_is_called_by_its_local_name_in_overlays_and_notices() {
        let mut app = app_for_the_switcher();
        app.active = 2;
        submit(&mut app, "/rename proj");

        submit(&mut app, "/who");
        assert_eq!(app.viewing_peers.as_ref().unwrap().heading, "#proj");
        app.handle_key(key(KeyCode::Esc));

        app.remove_channel("project-x");
        let last = as_system(app.active().messages.back().unwrap());
        assert_eq!(last, "left #proj");
    }

    #[test]
    fn renaming_again_replaces_the_previous_name() {
        let mut app = app_for_the_switcher();
        app.active = 2;

        submit(&mut app, "/rename proj");
        submit(&mut app, "/rename px");

        assert_eq!(app.channel_label("project-x"), "px");
    }

    #[test]
    fn a_local_name_may_have_spaces() {
        let mut app = app_for_the_switcher();
        app.active = 2;

        submit(&mut app, "/rename  my project ");

        assert_eq!(app.channel_label("project-x"), "my project");
    }

    #[test]
    fn a_bare_rename_restores_the_stored_name() {
        let mut app = app_for_the_switcher();
        app.active = 2;
        submit(&mut app, "/rename proj");

        let action = submit(&mut app, "/rename");

        match action.expect("restoring a name returns an action to persist that") {
            InputAction::Rename { channel, name } => {
                assert_eq!(channel, "project-x");
                assert_eq!(name, None);
            }
            _ => panic!("expected InputAction::Rename"),
        }
        assert_eq!(app.channel_label("project-x"), "project-x");
    }

    #[test]
    fn a_bare_rename_with_nothing_to_restore_shows_usage() {
        let mut app = app_for_the_switcher();
        app.active = 2;

        let action = submit(&mut app, "/rename");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("usage:"), "got: {last}");
    }

    #[test]
    fn rename_is_refused_on_a_dm_with_a_pointer_to_alias() {
        let mut app = app_for_the_switcher();
        app.active = 1; // the /msg DM

        let action = submit(&mut app, "/rename buddy");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("/alias"), "got: {last}");
        assert_eq!(app.channel_label("dm-Alice"), "Alice", "label untouched");
        // Restoring is refused the same way: there is nothing here to restore.
        assert!(submit(&mut app, "/rename").is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("/alias"), "got: {last}");
    }

    #[test]
    fn rename_is_refused_on_a_point_to_point_chat_too() {
        let mut app = app_for_the_switcher();
        app.active = 3; // the /dm chat

        let action = submit(&mut app, "/rename buddy");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("/alias"), "got: {last}");
        assert_eq!(app.channel_label("p2p-08080808"), "Bob", "label untouched");
    }

    #[test]
    fn rename_refuses_a_name_another_group_channel_already_shows() {
        let mut app = app_for_the_switcher();
        app.active = 2;
        submit(&mut app, "/rename work"); // project-x is now shown as `work`
        app.active = 0; // general

        let action = submit(&mut app, "/rename work");

        assert!(action.is_none());
        assert_eq!(app.channel_label("general"), "general");
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.contains("another channel"), "got: {last}");
    }

    #[test]
    fn rename_refuses_another_channels_stored_name_even_when_it_is_hidden() {
        // `/leave project-x` still matches that stored name first, so naming
        // another channel that would leave the two impossible to tell apart.
        let mut app = app_for_the_switcher();
        app.active = 2;
        submit(&mut app, "/rename work"); // `project-x` no longer on screen
        app.active = 0; // general

        let action = submit(&mut app, "/rename project-x");

        assert!(action.is_none());
        assert_eq!(app.channel_label("general"), "general");
    }

    #[test]
    fn a_channel_may_be_renamed_to_its_own_stored_name() {
        let mut app = app_for_the_switcher();
        app.active = 2;
        submit(&mut app, "/rename proj");

        let action = submit(&mut app, "/rename project-x");

        assert!(action.is_some());
        assert_eq!(app.channel_label("project-x"), "project-x");
    }

    #[test]
    fn a_group_channel_may_share_a_name_with_a_conversation() {
        let mut app = app_for_the_switcher(); // `Alice` is the DM's label

        let action = submit(&mut app, "/rename Alice"); // general is active

        assert!(action.is_some());
        assert_eq!(app.channel_heading("general"), "#Alice");
        assert_eq!(
            app.channel_heading("dm-Alice"),
            "@Alice",
            "the glyph tells the two apart"
        );
    }

    #[test]
    fn leaving_a_renamed_channel_forgets_its_name_so_a_rejoin_starts_clean() {
        let mut app = app_for_the_switcher();
        app.active = 2;
        submit(&mut app, "/rename proj");

        app.remove_channel("project-x");
        // Rejoined later under the very same stored name.
        app.handle_net_event(NetEvent::Joined("project-x".to_string()));

        assert_eq!(app.active().name, "project-x");
        assert_eq!(app.channel_label("project-x"), "project-x");
    }

    #[test]
    fn load_renames_seeds_local_names() {
        let mut app = app_for_the_switcher();

        app.load_renames(HashMap::from([(
            "project-x".to_string(),
            "proj".to_string(),
        )]));

        assert_eq!(app.channel_label("project-x"), "proj");
        assert_eq!(app.channel_label("general"), "general");
    }

    #[test]
    fn a_stored_rename_never_applies_to_a_conversation() {
        let mut app = app_for_the_switcher();

        app.load_renames(HashMap::from([
            ("dm-Alice".to_string(), "nope".to_string()),
            ("p2p-08080808".to_string(), "nope".to_string()),
        ]));

        assert_eq!(app.channel_label("dm-Alice"), "Alice");
        assert_eq!(app.channel_label("p2p-08080808"), "Bob");
    }

    #[test]
    fn leave_and_the_switcher_find_a_renamed_channel_by_its_new_name() {
        let mut app = app_for_the_switcher();
        app.active = 2;
        submit(&mut app, "/rename proj");
        app.active = 0;

        // `/leave` takes the new name, its heading, and still the stored name.
        for typed in ["proj", "#proj", "project-x"] {
            match submit(&mut app, &format!("/leave {typed}")) {
                Some(InputAction::Leave(channel)) => {
                    assert_eq!(channel, "project-x", "{typed}");
                }
                _ => panic!("expected InputAction::Leave for {typed}"),
            }
        }

        // The switcher matches what is shown, not the stored name.
        ctrl_p(&mut app);
        type_text(&mut app, "proj");
        assert_eq!(app.switcher_matches(), vec![2]);
        for _ in 0..4 {
            app.handle_key(key(KeyCode::Backspace));
        }
        type_text(&mut app, "-x");
        assert!(
            app.switcher_matches().is_empty(),
            "the stored name is not searched"
        );
    }

    // -- auto-save ------------------------------------------------------------
    // `/autosave` and `auto_save`. Most of these start from
    // `app_with_autosave`: `general` (active) with auto-save on and the files
    // root at `/files`.

    fn app_with_autosave() -> AppState {
        let mut app = app();
        app.load_files_root(PathBuf::from("/files"));
        app.load_autosave(HashSet::from(["general".to_string()]));
        app
    }

    /// `SaveMode` of every auto-save.
    fn automatic() -> SaveMode {
        SaveMode::Automatic {
            max_bytes: DEFAULT_AUTOSAVE_LIMIT,
        }
    }

    /// What main.rs does with a message off the network: hands it to the
    /// app and -- only if that made it a new line in the transcript -- asks
    /// what auto-save makes of it, with nothing yet saved on disk.
    fn arrive(app: &mut AppState, channel: &str, message: ChatMessage) -> Option<SaveRequest> {
        let (channel, message) =
            app.handle_net_event(NetEvent::Received(channel.to_string(), message))?;
        app.auto_save(&channel, &message, |_| false)
    }

    /// Gives `app` a user key of its own and returns it, so that devices can
    /// be certified under it.
    fn own_user_key(app: &mut AppState) -> SecretKey {
        let key = SecretKey::generate();
        app.load_self_user_id(*key.public().as_bytes());
        key
    }

    /// Certifies `device` under `key`, as its own announcement would.
    fn certify(app: &mut AppState, key: &SecretKey, device: [u8; 32]) {
        app.handle_net_event(NetEvent::Device(
            "general".to_string(),
            device_cert(key, device),
        ));
    }

    /// The newest line in `channel`'s transcript, which must be a notice.
    fn newest_notice<'a>(app: &'a AppState, channel: &str) -> &'a str {
        let channel = app
            .channels
            .iter()
            .find(|c| c.name == channel)
            .expect("a joined channel");
        as_system(channel.messages.back().expect("a line in the transcript"))
    }

    /// How many notices `channel`'s transcript holds.
    fn notices_in(app: &AppState, channel: &str) -> usize {
        app.channels
            .iter()
            .find(|c| c.name == channel)
            .expect("a joined channel")
            .messages
            .iter()
            .filter(|line| matches!(line, TranscriptLine::System(_)))
            .count()
    }

    /// A file message from `sender` that announces `size` bytes, whatever
    /// the file really holds.
    fn file_message_of_size(id: u64, sender: [u8; 32], filename: &str, size: u64) -> ChatMessage {
        let mut message = file_message(id, sender, filename, filename.as_bytes());
        message.attachment.as_mut().unwrap().size = size;
        message
    }

    /// The first eight hex digits of the hash `file_message` gives `content`,
    /// which is what a `/save` hint names.
    fn hint_prefix(content: &[u8]) -> String {
        hash_of(content).to_hex()[..8].to_string()
    }

    fn alice() -> HashMap<[u8; 32], String> {
        HashMap::from([(PEER_ID, "Alice".to_string())])
    }

    #[test]
    fn auto_save_is_off_until_it_is_turned_on_and_a_bare_command_says_so() {
        let mut app = app();
        app.load_files_root(PathBuf::from("/files"));

        let action = submit(&mut app, "/autosave");

        assert!(action.is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.starts_with("auto-save is off for #general"),
            "got: {last}"
        );
        assert!(last.contains("25.0 MB"), "the limit, got: {last}");
        assert!(last.contains("/files/general"), "the folder, got: {last}");
        assert!(
            last.contains("/autosave on"),
            "how to turn it on, got: {last}"
        );
    }

    #[test]
    fn autosave_on_switches_it_on_for_the_active_channel_and_asks_to_remember_it() {
        let mut app = app();

        match submit(&mut app, "/autosave on") {
            Some(InputAction::SetAutoSave { channel, on }) => {
                assert_eq!(channel, "general");
                assert!(on);
            }
            _ => panic!("expected InputAction::SetAutoSave"),
        }
        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.starts_with("auto-save is on for #general"),
            "got: {last}"
        );
        assert!(last.contains("/autosave off"), "got: {last}");

        // A bare command agrees.
        assert!(submit(&mut app, "/autosave").is_none());
        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.starts_with("auto-save is on for #general"),
            "got: {last}"
        );
    }

    #[test]
    fn outbox_on_requests_runtime_setup_without_claiming_success_early() {
        let mut app = app();
        app.load_files_root(PathBuf::from("/files"));

        match submit(&mut app, "/outbox on") {
            Some(InputAction::SetOutbox { channel, on }) => {
                assert_eq!(channel, "general");
                assert!(on);
            }
            _ => panic!("expected InputAction::SetOutbox"),
        }
        assert!(
            !app.outbox.contains("general"),
            "the runtime must create and persist the folder before state says it is on"
        );
    }

    #[test]
    fn outbox_status_is_per_channel_and_uses_the_send_subfolder() {
        let mut app = multi_channel_app();
        app.load_files_root(PathBuf::from("/files"));
        app.load_outbox(HashSet::from(["general".to_string()]));

        submit(&mut app, "/outbox"); // random is active
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("outbox is off for #random"), "got: {last}");
        assert!(last.contains("/files/random/send"), "got: {last}");

        app.active = 0;
        submit(&mut app, "/outbox");
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("outbox is on for #general"), "got: {last}");
    }

    #[test]
    fn a_bad_outbox_argument_is_usage_and_changes_nothing() {
        let mut app = app();

        assert!(submit(&mut app, "/outbox later").is_none());

        assert_eq!(
            as_system(app.active().messages.back().unwrap()),
            "usage: /outbox [on|off]"
        );
        assert!(app.outbox.is_empty());
    }

    #[test]
    fn autosave_off_switches_it_off_again_and_asks_to_remember_that_too() {
        let mut app = app_with_autosave();

        match submit(&mut app, "/autosave off") {
            Some(InputAction::SetAutoSave { channel, on }) => {
                assert_eq!(channel, "general");
                assert!(!on);
            }
            _ => panic!("expected InputAction::SetAutoSave"),
        }
        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.starts_with("auto-save is off for #general"),
            "got: {last}"
        );
    }

    #[test]
    fn asking_for_what_already_holds_remembers_nothing_but_still_says_it() {
        let mut app = app();

        assert!(submit(&mut app, "/autosave off").is_none(), "already off");
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("auto-save is off"), "got: {last}");

        assert!(submit(&mut app, "/autosave on").is_some());
        assert!(submit(&mut app, "/autosave on").is_none(), "already on");
        let last = as_system(app.active().messages.back().unwrap());
        assert!(last.starts_with("auto-save is on"), "got: {last}");
    }

    #[test]
    fn auto_save_is_per_channel() {
        let mut app = multi_channel_app(); // "random" is active

        submit(&mut app, "/autosave on");
        app.active = 0; // general
        submit(&mut app, "/autosave");

        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.starts_with("auto-save is off for #general"),
            "got: {last}"
        );
    }

    #[test]
    fn auto_save_can_be_switched_on_in_a_conversation_and_names_it_by_its_heading() {
        let mut app = app_for_the_switcher();
        app.active = 1; // the /msg DM, shown as Alice

        match submit(&mut app, "/autosave on") {
            Some(InputAction::SetAutoSave { channel, on }) => {
                assert_eq!(channel, "dm-Alice", "by stored name, like its folder");
                assert!(on);
            }
            _ => panic!("expected InputAction::SetAutoSave"),
        }
        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.starts_with("auto-save is on for @Alice"),
            "got: {last}"
        );
    }

    #[test]
    fn a_bad_autosave_argument_is_usage_and_changes_nothing() {
        let mut app = app();

        for typed in ["maybe", "on off", "ON"] {
            let action = submit(&mut app, &format!("/autosave {typed}"));

            assert!(action.is_none(), "{typed}");
            let last = as_system(app.active().messages.back().unwrap());
            assert_eq!(
                last, "usage: /autosave [on|off|limit <size|default>]",
                "{typed}"
            );
        }
        assert!(app.autosave.is_empty());
    }

    #[test]
    fn load_autosave_seeds_the_channels_that_have_it_on() {
        let mut app = multi_channel_app();

        app.load_autosave(HashSet::from(["general".to_string()]));

        submit(&mut app, "/autosave"); // random is active
        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.starts_with("auto-save is off for #random"),
            "got: {last}"
        );
        app.active = 0;
        submit(&mut app, "/autosave");
        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.starts_with("auto-save is on for #general"),
            "got: {last}"
        );
    }

    #[test]
    fn help_lists_autosave_and_its_limit() {
        let mut app = app();

        submit(&mut app, "/help");

        let last = as_system(app.active().messages.back().unwrap());
        assert!(
            last.contains("/autosave [on|off|limit <size|default>]"),
            "got: {last}"
        );
        assert!(last.contains("device-wide"), "got: {last}");
        assert!(last.contains("channel overrides it"), "got: {last}");
    }

    #[test]
    fn a_file_from_one_of_our_own_devices_is_saved() {
        let mut app = app_with_autosave();
        let key = own_user_key(&mut app);
        certify(&mut app, &key, PEER_ID);

        let request = arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        )
        .expect("our own device is trusted");

        assert_eq!(
            request,
            SaveRequest {
                channel: "general".to_string(),
                hash: hash_of(b"report"),
                filename: "report.pdf".to_string(),
                providers: vec![PEER_ID],
                dir: PathBuf::from("/files/general"),
                mode: automatic(),
            }
        );
        assert_eq!(
            notices_in(&app, "general"),
            0,
            "nothing to say until it has landed"
        );
    }

    #[test]
    fn a_file_from_a_peer_we_have_aliased_is_saved() {
        let mut app = app_with_autosave();
        app.load_contacts(alice());

        let request = arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );

        assert_eq!(request.expect("Alice is aliased").providers, vec![PEER_ID]);
    }

    #[test]
    fn an_alias_covers_every_device_of_that_person() {
        let mut app = app_with_autosave();
        let their_key = SecretKey::generate();
        for device in [PEER_ID, PEER_ID_2] {
            app.handle_net_event(NetEvent::Device(
                "general".to_string(),
                device_cert(&their_key, device),
            ));
        }
        // Given to one of their devices; kept under who they are.
        app.load_contacts(HashMap::from([(
            app.canonical_id(&PEER_ID),
            "Jurgen".to_string(),
        )]));

        let request = arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID_2, "report.pdf", b"report"),
        );

        assert!(request.is_some());
    }

    #[test]
    fn a_file_from_a_stranger_is_skipped_and_says_why() {
        let mut app = app_with_autosave();

        let request = arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );

        assert!(request.is_none());
        let notice = newest_notice(&app, "general");
        assert!(
            notice.starts_with("not auto-saving report.pdf from "),
            "got: {notice}"
        );
        assert!(notice.contains(&hex_prefix(&PEER_ID)), "who, got: {notice}");
        assert!(
            notice.contains("your own devices and the peers you have aliased"),
            "why, got: {notice}"
        );
        assert!(
            notice.contains(&format!(
                "/save {} to download it yourself",
                hint_prefix(b"report")
            )),
            "the way out, got: {notice}"
        );
    }

    #[test]
    fn a_device_we_forgot_is_no_longer_one_of_our_own() {
        let mut app = app_with_autosave();
        let key = own_user_key(&mut app);
        certify(&mut app, &key, PEER_ID);
        app.load_forgotten_devices(HashSet::from([PEER_ID]));

        let request = arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );

        assert!(request.is_none());
        assert!(newest_notice(&app, "general").starts_with("not auto-saving report.pdf"));
    }

    #[test]
    fn a_file_from_ourselves_is_never_auto_saved_and_nothing_is_said() {
        let mut app = app_with_autosave();

        let request = arrive(
            &mut app,
            "general",
            file_message(1, SELF_ID, "report.pdf", b"report"),
        );

        assert!(request.is_none());
        assert_eq!(notices_in(&app, "general"), 0);
    }

    #[test]
    fn a_message_without_a_file_is_none_of_auto_saves_business() {
        let mut app = app_with_autosave();
        app.load_contacts(alice());

        let request = arrive(&mut app, "general", compose_message(PEER_ID, "hello", None));

        assert!(request.is_none());
        assert_eq!(notices_in(&app, "general"), 0);
    }

    #[test]
    fn nothing_is_saved_or_said_while_auto_save_is_off() {
        let mut app = app();
        app.load_contacts(alice());

        let request = arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );

        assert!(request.is_none());
        assert_eq!(notices_in(&app, "general"), 0);
    }

    #[test]
    fn a_file_over_the_limit_is_skipped_with_its_size_and_the_way_to_fetch_it() {
        let mut app = app_with_autosave();
        app.load_contacts(alice());
        let message = file_message_of_size(1, PEER_ID, "video.mov", 80 * 1024 * 1024);
        let prefix = message.attachment.as_ref().unwrap().hash.to_hex()[..8].to_string();

        let request = arrive(&mut app, "general", message);

        assert!(request.is_none());
        let notice = newest_notice(&app, "general");
        assert!(notice.contains("video.mov (80.0 MB)"), "got: {notice}");
        assert!(
            notice.contains("over the 25.0 MB auto-save limit"),
            "got: {notice}"
        );
        assert!(
            notice.contains(&format!("/save {prefix} to download it yourself")),
            "got: {notice}"
        );
    }

    #[test]
    fn a_file_exactly_at_the_limit_is_saved_and_one_byte_more_is_not() {
        let mut app = app_with_autosave();
        app.load_contacts(alice());

        let at = file_message_of_size(1, PEER_ID, "at.bin", DEFAULT_AUTOSAVE_LIMIT);
        let over = file_message_of_size(2, PEER_ID, "over.bin", DEFAULT_AUTOSAVE_LIMIT + 1);

        assert!(arrive(&mut app, "general", at).is_some());
        assert!(arrive(&mut app, "general", over).is_none());
    }

    #[test]
    fn a_file_the_channel_already_holds_is_skipped_and_says_so() {
        let mut app = app_with_autosave();
        app.load_contacts(alice());
        let (channel, message) = app
            .handle_net_event(NetEvent::Received(
                "general".to_string(),
                file_message(1, PEER_ID, "report.pdf", b"report"),
            ))
            .unwrap();

        let request = app.auto_save(&channel, &message, |hash| hash == hash_of(b"report"));

        assert!(request.is_none());
        let notice = newest_notice(&app, "general");
        assert!(
            notice.contains("not auto-saving report.pdf")
                && notice.contains("already has a saved copy"),
            "got: {notice}"
        );
    }

    #[test]
    fn a_repeat_of_a_save_in_flight_is_dropped_until_it_finishes() {
        let mut app = app_with_autosave();
        app.load_contacts(alice());
        assert!(
            arrive(
                &mut app,
                "general",
                file_message(1, PEER_ID, "report.pdf", b"report")
            )
            .is_some()
        );

        // The same file again -- sent twice, or a share replayed -- while the
        // first is still downloading: one download is enough.
        let again = arrive(
            &mut app,
            "general",
            file_message(2, PEER_ID, "report.pdf", b"report"),
        );
        assert!(again.is_none());
        assert_eq!(
            notices_in(&app, "general"),
            0,
            "dropped quietly: the first one's notice is on its way"
        );

        // Once it has landed it is a new matter (main.rs asks the disk first).
        app.handle_net_event(NetEvent::FileSaved {
            channel: "general".to_string(),
            hash: hash_of(b"report"),
            filename: "report.pdf".to_string(),
            path: PathBuf::from("/files/general/report.pdf"),
            mode: automatic(),
        });
        let later = arrive(
            &mut app,
            "general",
            file_message(3, PEER_ID, "report.pdf", b"report"),
        );
        assert!(later.is_some());
    }

    #[test]
    fn a_save_that_failed_can_be_tried_again_by_the_next_share() {
        let mut app = app_with_autosave();
        app.load_contacts(alice());
        assert!(
            arrive(
                &mut app,
                "general",
                file_message(1, PEER_ID, "report.pdf", b"report")
            )
            .is_some()
        );

        app.save_failed(
            "general",
            hash_of(b"report"),
            "report.pdf",
            "boom",
            automatic(),
        );

        let next = arrive(
            &mut app,
            "general",
            file_message(2, PEER_ID, "report.pdf", b"report"),
        );
        assert!(next.is_some());
    }

    #[test]
    fn a_redelivered_message_is_not_auto_saved_a_second_time() {
        let mut app = app_with_autosave();
        app.load_contacts(alice());
        let message = file_message(1, PEER_ID, "report.pdf", b"report");
        assert!(arrive(&mut app, "general", message.clone()).is_some());
        app.handle_net_event(NetEvent::FileSaved {
            channel: "general".to_string(),
            hash: hash_of(b"report"),
            filename: "report.pdf".to_string(),
            path: PathBuf::from("/files/general/report.pdf"),
            mode: automatic(),
        });

        // The very same message again: not a new line, so never asked about.
        assert!(arrive(&mut app, "general", message).is_none());
    }

    #[test]
    fn a_file_is_only_ever_asked_of_peers_we_trust() {
        let mut app = app_with_autosave();
        app.load_contacts(HashMap::from([
            (PEER_ID, "Alice".to_string()),
            (PEER_ID_3, "Carol".to_string()),
        ]));
        // A stranger who is online and has posted, listed ahead of Carol.
        receive(
            &mut app,
            "general",
            compose_message(PEER_ID_2, "hello", None),
        );
        peers_join(&mut app, "general", &[PEER_ID_2, PEER_ID_3]);

        let request = arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        )
        .unwrap();

        assert_eq!(request.providers, vec![PEER_ID, PEER_ID_3]);
    }

    #[test]
    fn our_other_devices_that_are_online_are_asked_too() {
        let mut app = app_with_autosave();
        let key = own_user_key(&mut app);
        certify(&mut app, &key, PEER_ID);
        certify(&mut app, &key, PEER_ID_2);
        peers_join(&mut app, "general", &[PEER_ID_2]);

        let request = arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        )
        .unwrap();

        assert_eq!(request.providers, vec![PEER_ID, PEER_ID_2]);
    }

    #[test]
    fn a_share_forged_as_a_trusted_peer_still_cannot_reach_a_stranger() {
        // Nothing signs a chat message, so a stranger in the channel can claim
        // to be Alice. What it can't do is be asked for the file: the only
        // peers asked are the ones we trust, known by their connection.
        let mut app = app_with_autosave();
        app.load_contacts(alice());
        peers_join(&mut app, "general", &[PEER_ID_2]); // the stranger, online
        let forged = file_message(1, PEER_ID, "invoice.pdf", b"the stranger's own bytes");

        let request = arrive(&mut app, "general", forged).expect("claims to be Alice: tried");

        assert_eq!(
            request.providers,
            vec![PEER_ID],
            "only the real Alice is asked, and she doesn't have it"
        );
    }

    #[test]
    fn a_file_goes_into_the_folder_of_the_channel_it_arrived_in_whichever_is_active() {
        let mut app = multi_channel_app(); // "random" is active
        app.load_files_root(PathBuf::from("/files"));
        app.load_autosave(HashSet::from(["general".to_string()]));
        app.load_contacts(alice());

        let request = arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        )
        .unwrap();

        assert_eq!(request.channel, "general");
        assert_eq!(request.dir, PathBuf::from("/files/general"));
    }

    #[test]
    fn what_auto_save_says_lands_in_the_channel_the_file_arrived_in() {
        let mut app = multi_channel_app(); // "random" is active
        app.load_autosave(HashSet::from(["general".to_string()]));

        arrive(
            &mut app,
            "general",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );

        assert!(newest_notice(&app, "general").starts_with("not auto-saving report.pdf"));
        assert_eq!(
            notices_in(&app, "random"),
            0,
            "the channel being looked at hears nothing"
        );
    }

    #[test]
    fn an_automatic_save_is_reported_in_its_own_channel_and_a_manual_one_where_you_are() {
        let mut app = multi_channel_app(); // "random" is active
        let saved = |mode| NetEvent::FileSaved {
            channel: "general".to_string(),
            hash: hash_of(b"report"),
            filename: "report.pdf".to_string(),
            path: PathBuf::from("/files/general/report.pdf"),
            mode,
        };

        app.handle_net_event(saved(automatic()));
        assert_eq!(
            newest_notice(&app, "general"),
            "auto-saved report.pdf to /files/general/report.pdf"
        );
        assert_eq!(notices_in(&app, "random"), 0);

        app.handle_net_event(saved(SaveMode::Manual));
        assert_eq!(
            newest_notice(&app, "random"),
            "saved report.pdf to /files/general/report.pdf"
        );
    }

    #[test]
    fn a_failed_save_says_so_in_the_words_of_its_mode() {
        let mut app = multi_channel_app(); // "random" is active
        let hash = hash_of(b"report");

        app.save_failed("general", hash, "report.pdf", "boom", SaveMode::Manual);
        assert_eq!(
            newest_notice(&app, "random"),
            "failed to save report.pdf: boom"
        );

        app.save_failed("general", hash, "report.pdf", "boom", automatic());
        assert_eq!(
            newest_notice(&app, "general"),
            format!(
                "couldn't auto-save report.pdf: boom -- /save {} to download it yourself",
                hint_prefix(b"report")
            )
        );
        assert_eq!(notices_in(&app, "random"), 1, "only the manual one");
    }

    #[test]
    fn leaving_a_channel_forgets_its_auto_save_so_a_later_room_of_that_name_starts_off() {
        let mut app = app_for_the_switcher(); // Alice (PEER_ID) is aliased
        app.load_autosave(HashSet::from(["project-x".to_string()]));

        app.remove_channel("project-x");
        // A different room, rejoined under the very same stored name.
        app.handle_net_event(NetEvent::Joined("project-x".to_string()));
        assert_eq!(app.active().name, "project-x");

        let request = arrive(
            &mut app,
            "project-x",
            file_message(1, PEER_ID, "report.pdf", b"report"),
        );

        assert!(
            request.is_none(),
            "it must not inherit the old room's setting"
        );
        assert_eq!(notices_in(&app, "project-x"), 0);
        submit(&mut app, "/autosave");
        assert!(newest_notice(&app, "project-x").starts_with("auto-save is off"));
    }

    #[test]
    fn leaving_a_channel_drops_its_saves_in_flight() {
        let mut app = app_for_the_switcher(); // Alice (PEER_ID) is aliased
        app.load_autosave(HashSet::from(["project-x".to_string()]));
        let file = || file_message(1, PEER_ID, "report.pdf", b"report");
        assert!(arrive(&mut app, "project-x", file()).is_some());

        app.remove_channel("project-x");
        app.handle_net_event(NetEvent::Joined("project-x".to_string()));
        submit(&mut app, "/autosave on");

        let request = arrive(
            &mut app,
            "project-x",
            file_message(2, PEER_ID, "report.pdf", b"report"),
        );
        assert!(request.is_some(), "no stale entry may block it");
    }

    #[test]
    fn channel_auto_save_limit_overrides_device_limit_and_is_forgotten_on_leave() {
        let mut app = app_for_the_switcher();
        app.load_autosave_limit(AutoSaveLimit::Bytes(1024));
        app.load_autosave_limits(HashMap::from([(
            "project-x".into(),
            AutoSaveLimit::Bytes(2048),
        )]));
        app.load_autosave(HashSet::from(["project-x".to_string()]));
        app.load_contacts(alice());

        assert!(
            arrive(
                &mut app,
                "project-x",
                file_message_of_size(1, PEER_ID, "large.bin", 1500)
            )
            .is_some()
        );
        app.remove_channel("project-x");
        assert!(!app.autosave_limits.contains_key("project-x"));
    }

    #[test]
    fn sync_works_on_a_msg_dm_channel() {
        let mut app = app_with_dm("dm-Alice", PEER_ID);
        app.active = 1;

        let action = submit(&mut app, "/sync publish-and-receive").unwrap();

        let InputAction::SetSync { channel, config } = action else {
            panic!("expected sync persistence action");
        };
        assert_eq!(channel, "dm-Alice");
        assert_eq!(config.policy, SyncPolicy::PublishAndReceive);
    }

    #[test]
    fn sync_works_on_a_point_to_point_channel() {
        let mut app = app();
        app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);

        let action = submit(&mut app, "/sync on").unwrap();

        let InputAction::SetSync { channel, config } = action else {
            panic!("expected sync persistence action");
        };
        assert_eq!(channel, "p2p-alice");
        assert_eq!(config.policy, SyncPolicy::PublishAndReceive);
    }

    #[test]
    fn a_bare_sync_explains_setup_before_anything_is_configured() {
        let mut app = app();

        submit(&mut app, "/sync");

        let notice = as_system(app.active().messages.back().unwrap());
        assert!(notice.contains("/invite"), "got: {notice}");
        assert!(notice.contains("/join"), "got: {notice}");
        assert!(
            !notice.contains("pending"),
            "an unconfigured channel should not look like an active status report: {notice}"
        );
    }

    #[test]
    fn a_bare_sync_explains_setup_differently_for_a_point_to_point_chat() {
        // Already has its one peer, so there is no invited group to join.
        let mut app = app();
        app.activate_direct_channel("p2p-alice".to_string(), PEER_ID);

        submit(&mut app, "/sync");

        let notice = as_system(app.active().messages.back().unwrap());
        assert!(
            !notice.contains("/invite"),
            "a point-to-point chat has no invite step: {notice}"
        );
    }

    #[test]
    fn a_bare_sync_reports_status_once_sync_has_been_configured() {
        let mut app = app();
        submit(&mut app, "/sync receive-only");

        submit(&mut app, "/sync");

        let notice = as_system(app.active().messages.back().unwrap());
        assert!(notice.contains("pending"), "got: {notice}");
        assert!(!notice.contains("/invite"), "got: {notice}");
    }

    #[test]
    fn a_sync_report_with_conflicts_points_to_how_to_resolve_them() {
        let mut app = app();
        submit(&mut app, "/sync on");
        app.sync_conflict(
            "general",
            SyncConflict {
                local_path: "report.txt".into(),
                remote_path: "report.txt".into(),
                remote_copy_path: Some("report (remote conflict 1).txt".into()),
                local_modified_unix_ms: None,
                remote_modified_unix_ms: None,
                author: None,
            },
        );

        submit(&mut app, "/sync");

        let notice = as_system(app.active().messages.back().unwrap());
        assert!(
            notice.contains("/conflicts local|remote|both"),
            "got: {notice}"
        );
    }

    #[test]
    fn a_sync_report_with_pending_files_points_to_who() {
        let mut app = app();
        submit(&mut app, "/sync on");
        app.sync_pending("general", "report.txt".to_string());

        submit(&mut app, "/sync");

        let notice = as_system(app.active().messages.back().unwrap());
        assert!(notice.contains("/who"), "got: {notice}");
    }

    #[test]
    fn sync_policies_are_independent_per_channel() {
        let mut app = AppState::new(
            SELF_ID,
            vec!["general".to_string(), "project".to_string()],
            "general",
        );

        let action = submit(&mut app, "/sync receive-only").unwrap();
        let InputAction::SetSync { channel, config } = action else {
            panic!("expected sync persistence action");
        };
        assert_eq!(channel, "general");
        assert_eq!(config.policy, SyncPolicy::ReceiveOnly);
        assert!(app.sync_receives("general"));

        app.active = 1;
        let action = submit(&mut app, "/sync publish-and-receive").unwrap();
        let InputAction::SetSync { channel, config } = action else {
            panic!("expected sync persistence action");
        };
        assert_eq!(channel, "project");
        assert_eq!(config.policy, SyncPolicy::PublishAndReceive);
        assert_eq!(app.sync_config("general").policy, SyncPolicy::ReceiveOnly);
    }

    #[test]
    fn sync_rules_are_persisted_actions_and_visible_in_status() {
        let mut app = app();

        let action = submit(&mut app, "/sync include docs/**").unwrap();
        let InputAction::SetSync { config, .. } = action else {
            panic!("expected sync persistence action");
        };
        assert_eq!(config.rules, vec![SyncRule::include("docs/**").unwrap()]);
        assert!(app.sync_allows_path("general", "docs/plan.md"));
        assert!(!app.sync_allows_path("general", "images/logo.png"));

        submit(&mut app, "/sync exclude docs/private/**");
        submit(&mut app, "/sync");
        let status = as_system(app.active().messages.back().unwrap());
        assert!(status.contains("include docs/**"), "{status}");
        assert!(status.contains("exclude docs/private/**"), "{status}");
        assert!(!app.sync_allows_path("general", "docs/private/plan.md"));
    }

    #[test]
    fn sync_pause_resumes_the_selected_policy() {
        let mut app = app();
        submit(&mut app, "/sync receive-only");
        submit(&mut app, "/sync pause");
        assert!(app.sync_paused("general"));
        assert!(!app.sync_receives("general"));

        let action = submit(&mut app, "/sync resume").unwrap();
        let InputAction::SetSync { config, .. } = action else {
            panic!("expected sync persistence action");
        };
        assert_eq!(config.policy, SyncPolicy::ReceiveOnly);
        assert!(!config.paused);
    }

    #[test]
    fn sync_rejects_an_invalid_rule_without_changing_configuration() {
        let mut app = app();

        let action = submit(&mut app, "/sync include ../outside");

        assert!(action.is_none());
        assert!(app.sync_config("general").rules.is_empty());
        assert!(as_system(app.active().messages.back().unwrap()).contains("relative glob"));
    }

    #[test]
    fn trash_commands_target_only_the_active_channel() {
        let mut app = AppState::new(
            SELF_ID,
            vec!["general".to_string(), "project".to_string()],
            "general",
        );
        app.active = 1;

        assert!(matches!(
            submit(&mut app, "/trash"),
            Some(InputAction::ListSyncTrash(channel)) if channel == "project"
        ));
        assert!(matches!(
            submit(&mut app, "/trash restore old/report.txt"),
            Some(InputAction::RestoreSyncTrash { channel, path })
                if channel == "project" && path == "old/report.txt"
        ));
        assert!(matches!(
            submit(&mut app, "/trash purge"),
            Some(InputAction::PurgeSyncTrash(channel)) if channel == "project"
        ));
    }

    #[test]
    fn trash_rejects_missing_restore_path_and_unknown_arguments() {
        let mut app = app();

        assert!(submit(&mut app, "/trash restore").is_none());
        assert!(as_system(app.active().messages.back().unwrap()).starts_with("usage:"));
        assert!(submit(&mut app, "/trash tomorrow").is_none());
        assert!(as_system(app.active().messages.back().unwrap()).starts_with("usage:"));
    }

    #[test]
    fn conflicts_list_provenance_and_clear_only_after_resolution() {
        let mut app = app();
        submit(&mut app, "/sync on");
        let conflict = SyncConflict {
            local_path: "report.txt".into(),
            remote_path: "report.txt".into(),
            remote_copy_path: Some("report (remote conflict 1).txt".into()),
            local_modified_unix_ms: Some(10),
            remote_modified_unix_ms: Some(20),
            author: Some(PEER_ID),
        };
        app.sync_conflict("general", conflict.clone());
        assert_eq!(app.sync_marker("general"), Some('C'));

        submit(&mut app, "/conflicts");
        let report = as_system(app.active().messages.back().unwrap());
        assert!(report.contains("/conflicts local|remote|both"));
        assert!(
            app.sync_status("general")
                .is_some_and(|status| status.conflict_count() == 1)
        );

        assert!(matches!(
            submit(&mut app, "/conflicts remote report.txt"),
            Some(InputAction::ResolveSyncConflict {
                resolution: ConflictResolution::Remote,
                ..
            })
        ));
        assert_eq!(app.sync_marker("general"), Some('C'));
        app.sync_conflict_resolved("general", &conflict);
        assert_eq!(app.sync_marker("general"), Some('S'));
    }
}
