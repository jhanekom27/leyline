mod app;
mod autosave;
mod backfill;
mod channel_registry;
mod channel_renames;
mod contacts;
mod direct;
mod direct_registry;
mod dm_registry;
mod files;
mod forgotten_devices;
mod hyperlink;
mod identity;
mod last_seen;
mod markdown;
mod message;
mod net;
mod outbox;
mod pairing;
mod picker;
mod runtime;
mod saved_files;
mod search;
mod settings;
mod shared_files;
mod storage;
mod sync_activity;
mod sync_apply;
#[cfg(test)]
mod sync_docs;
mod sync_index;
mod sync_publish;
mod sync_settings;
mod sync_state;
mod term_events;
mod thread;
mod ticket;
mod ui;
mod version;

use std::collections::{HashMap, VecDeque};
use std::fs::OpenOptions;
use std::future::Future;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use arboard::Clipboard;
use crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use crossterm::execute;
use iroh_tickets::Ticket;
use tokio::sync::mpsc;
use tokio::time::{interval, timeout};
use tracing::{debug, warn};

use app::{
    AppState, ConflictResolution, FilesListing, InputAction, LocalFile, SaveRequest, SyncConflict,
};
use autosave::{AutoSave, limit as autosave_limit};
use backfill::BackfillStore;
use channel_registry::ChannelRegistry;
use channel_renames::ChannelRenames;
use contacts::Contacts;
use direct_registry::DirectRegistry;
use dm_registry::DmRegistry;
use files::{
    dropped_files, edit_in_text_editor, open_in_default_app, resolve_destination,
    reveal_in_file_manager, set_files_root, set_size_limit, stage_file, stage_image,
};
use forgotten_devices::ForgottenDevices;
use last_seen::LastSeen;
use message::ChatMessage;
use net::{DEVICE_SYNC_CHANNEL, Net, NetEvent};
use outbox::{Outbox, Watcher as OutboxWatcher};
use picker::{FolderListing, read_folder, remember_folder, start_folder};
use runtime::*;
use saved_files::SavedFiles;
use search::{SearchOutcome, run_on_disk_scan};
use settings::Settings;
use shared_files::SharedFiles;
use storage::MessageStore;
use sync_activity::{Activity, SyncActivity};
use sync_settings::SyncSettings;
use term_events::TermEvents;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // A bare `--version`/`-V` short-circuits everything else -- no need to
    // touch disk, identity, or networking just to print a version string.
    if std::env::args().any(|arg| arg == "--version" || arg == "-V") {
        println!("leyline {} ({})", version::VERSION, version::GIT_HASH);
        return Ok(());
    }

    let dirs = project_dirs()?;
    init_logging(&dirs)?;
    let data_dir = dirs.data_dir().to_path_buf();

    let store = MessageStore::new(dirs.data_dir().join("messages"))
        .context("failed to initialize message storage")?;
    let backfill = BackfillStore::new(dirs.data_dir().join("blobs"))
        .await
        .context("failed to initialize history backfill storage")?;
    let mut registry = ChannelRegistry::load(dirs.data_dir().join("channels"))
        .context("failed to initialize channel registry")?;
    let contacts = Contacts::load(dirs.data_dir().join("contacts"))
        .context("failed to initialize contacts")?;
    let last_seen = LastSeen::load(dirs.data_dir().join("last_seen"))
        .context("failed to initialize last-seen peer storage")?;
    let channel_renames = ChannelRenames::load(dirs.data_dir().join("channel_renames"))
        .context("failed to initialize channel renames")?;
    let autosave = AutoSave::load(dirs.data_dir().join("autosave"))
        .context("failed to initialize auto-save settings")?;
    let autosave_limit = autosave_limit(&data_dir)?;
    let outbox = Outbox::load(dirs.data_dir().join("outbox"))
        .context("failed to initialize outbox settings")?;
    let sync_settings = SyncSettings::load(dirs.data_dir().join("sync"))
        .context("failed to initialize sync settings")?;
    let sync_activity = SyncActivity::load(dirs.data_dir().join("sync_activity"))
        .context("failed to initialize sync activity")?;
    let dm_registry = DmRegistry::load(dirs.data_dir().join("dms"))
        .context("failed to initialize DM registry")?;
    let direct_registry = DirectRegistry::load(dirs.data_dir().join("direct_peers"))
        .context("failed to initialize direct-peer registry")?;
    let settings = Settings::load(dirs.data_dir().join("settings"))
        .context("failed to initialize settings")?;
    let forgotten_devices = ForgottenDevices::load(dirs.data_dir().join("forgotten_devices"))
        .context("failed to initialize forgotten devices")?;
    let files_root = files::files_root(&data_dir)?;
    let size_limit = files::size_limit(&data_dir)?;
    let saved_files = SavedFiles::load(dirs.data_dir().join("saved_files"))
        .context("failed to initialize saved files")?;
    let shared_files = SharedFiles::load(dirs.data_dir().join("shared_files"))
        .context("failed to initialize shared-file index")?;

    let secret_key = identity::load_or_generate(&dirs.config_dir().join("identity"))
        .context("failed to load or generate identity")?;
    let args = parse_args()?;
    let user_key_path = dirs.config_dir().join("user_key");
    if let Some(ticket) = &args.pair {
        pairing::bootstrap(&secret_key, ticket, &user_key_path, &mut registry)
            .await
            .context("failed to redeem pairing ticket")?;
    }
    let user_key = identity::load_or_generate(&user_key_path)
        .context("failed to load or generate user key")?;

    // What to rejoin from the previous session -- see channel_registry.rs
    // and `Net::start`. Empty on a first run (just "general" then) --
    // unless `--pair` just seeded it with another device's channels.
    let known_channels = registry.snapshot();

    let (net_tx, net_rx) = mpsc::channel(64);
    let replay_tx = net_tx.clone();
    let (net, joined_channels, active_channel) = Net::start(
        secret_key,
        user_key,
        args.join,
        known_channels,
        data_dir.join("docs"),
        settings.nickname(),
        net_tx,
        backfill.clone(),
    )
    .await
    .context("failed to start networking")?;

    let session = Session {
        net,
        net_rx,
        replay_tx,
        joined_channels,
        active_channel,
        store,
        backfill,
        registry,
        contacts,
        last_seen,
        channel_renames,
        autosave,
        autosave_limit,
        outbox,
        sync_settings,
        sync_activity,
        dm_registry,
        direct_registry,
        settings,
        forgotten_devices,
        saved_files,
        shared_files,
        files_root,
        size_limit,
        data_dir,
    };

    let mut terminal = ratatui::init();
    // ratatui's hook restores raw/alternate-screen modes. Stop mouse
    // reporting first, so a panic cannot leave the shell capturing input.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = write_mouse_reporting(&mut std::io::stdout(), false);
        previous_hook(info);
    }));
    // Bracketed paste lets a terminal paste arrive as one `Event::Paste`
    // instead of a burst of individual key events -- see
    // `AppState::paste_text` for why that matters. Best-effort: if the
    // terminal doesn't understand the enabling sequence, pastes just fall
    // back to today's character-by-character key events.
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let result = run(&mut terminal, session).await;
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    if let Err(err) = drain_terminal_events(event::poll, event::read) {
        debug!("failed to drain terminal events during shutdown: {err}");
    }
    ratatui::restore();
    match result {
        // A live `/pair <ticket>` redeem was confirmed (see
        // `app::AppState::restart_with_ticket`) -- the TUI has already
        // shut down cleanly above, so it's safe to wipe the state that
        // would otherwise conflict with the incoming identity and restart
        // into `--pair <ticket>`.
        Ok(Some(ticket)) => restart_for_pairing(&dirs, &ticket),
        Ok(None) => Ok(()),
        Err(err) => Err(err),
    }
}

/// Consumes events queued by the terminal after the app's event stream has
/// stopped. A terminal can have SGR mouse reports in flight when capture is
/// disabled; leaving them unread hands their escape sequence to the shell
/// after raw mode is restored.
///
/// The caller must have dropped any `EventStream` first: crossterm permits
/// only one event reader. The first short poll lets its background reader
/// release the terminal; later polls return immediately.
fn drain_terminal_events<P, R>(mut poll: P, mut read: R) -> std::io::Result<usize>
where
    P: FnMut(Duration) -> std::io::Result<bool>,
    R: FnMut() -> std::io::Result<Event>,
{
    let mut drained = 0;
    let mut timeout = Duration::from_millis(10);
    while poll(timeout)? {
        read()?;
        drained += 1;
        timeout = Duration::ZERO;
    }
    Ok(drained)
}
/// Mouse reporting follows the hints panel and is restored on every
/// return path (including draw or event-stream errors).
struct MouseReporting<W: Write> {
    writer: W,
    enabled: bool,
}

fn write_mouse_reporting(writer: &mut impl Write, enabled: bool) -> std::io::Result<()> {
    if enabled {
        execute!(writer, EnableMouseCapture, EnableFocusChange)
    } else {
        execute!(writer, DisableFocusChange, DisableMouseCapture)
    }
}

impl<W: Write> MouseReporting<W> {
    fn new(writer: W) -> Self {
        Self {
            writer,
            enabled: false,
        }
    }

    fn update(&mut self, enabled: bool) -> std::io::Result<()> {
        if self.enabled != enabled {
            // Set before writing so Drop also cleans up a partial enable.
            if enabled {
                self.enabled = true;
            }
            write_mouse_reporting(&mut self.writer, enabled)?;
            self.enabled = enabled;
        }
        Ok(())
    }
}

impl<W: Write> Drop for MouseReporting<W> {
    fn drop(&mut self) {
        if self.enabled {
            let _ = write_mouse_reporting(&mut self.writer, false);
        }
    }
}

fn queue_paused_sync_event(
    queued: &mut HashMap<String, VecDeque<NetEvent>>,
    channel: String,
    event: NetEvent,
) {
    let events = queued.entry(channel).or_default();
    if events.len() == PAUSED_SYNC_EVENT_LIMIT {
        events.pop_front();
    }
    events.push_back(event);
}

fn replay_paused_sync_events(
    queued: &mut HashMap<String, VecDeque<NetEvent>>,
    app: &AppState,
    tx: &mpsc::Sender<NetEvent>,
) {
    for (channel, events) in queued.iter_mut() {
        if !app.sync_receives(channel) {
            continue;
        }
        while let Some(event) = events.pop_front() {
            match tx.try_send(event) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(event)) => {
                    events.push_front(event);
                    break;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return,
            }
        }
    }
    queued.retain(|_, events| !events.is_empty());
}

/// Parsed CLI arguments: an optional `--join <ticket>` to bootstrap into
/// another instance's channel, and/or an optional `--pair <ticket>` to
/// bootstrap this device's whole identity from an already-paired one
/// (see `pairing::bootstrap`) before anything else starts. The two are
/// mutually exclusive -- `--pair` runs before this device has any
/// identity to join a channel under, so combining them in one invocation
/// isn't a case worth supporting yet.
struct Args {
    join: Option<String>,
    pair: Option<String>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = std::env::args().skip(1);
    let mut join = None;
    let mut pair = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--join" => {
                anyhow::ensure!(join.is_none(), "--join can only be specified once");
                join = Some(args.next().context("--join requires a ticket")?);
            }
            "--pair" => {
                anyhow::ensure!(pair.is_none(), "--pair can only be specified once");
                pair = Some(args.next().context("--pair requires a ticket")?);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    anyhow::ensure!(
        join.is_none() || pair.is_none(),
        "--join and --pair cannot be combined"
    );
    Ok(Args { join, pair })
}

/// Local state that would otherwise conflict with or leak into the
/// identity a confirmed live `/pair <ticket>` redeem is about to adopt --
/// see `restart_for_pairing`. Paths are relative to `data_dir`/
/// `config_dir` exactly as the rest of main.rs already resolves them
/// (`MessageStore::new`, `ChannelRegistry::load`, etc.) -- kept as one
/// explicit list, rather than wiping either directory wholesale, so this
/// can never reach `identity` (this device's own keypair -- pairing
/// changes the shared user key, not the device itself) or `leyline.log`,
/// which on some platforms (macOS) live in the very same directory as
/// everything here.
fn paths_to_wipe_for_pairing(data_dir: &Path, config_dir: &Path) -> Vec<PathBuf> {
    vec![
        config_dir.join("user_key"),
        data_dir.join("channels"),
        data_dir.join("messages"),
        data_dir.join("blobs"),
        data_dir.join("docs"),
        data_dir.join("contacts"),
        data_dir.join("channel_renames"),
        data_dir.join("autosave"),
        data_dir.join("outbox"),
        data_dir.join("dms"),
        data_dir.join("direct_peers"),
        data_dir.join("settings"),
        data_dir.join("forgotten_devices"),
    ]
}

/// Removes a file or directory at `path`, tolerating it already being
/// absent -- e.g. a local cache (`contacts`, `forgotten_devices`) that was
/// never written because the corresponding feature was never used on this
/// device.
fn remove_path(path: &Path) -> std::io::Result<()> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

/// Wipes every path `paths_to_wipe_for_pairing` names, stopping at the
/// first failure rather than continuing best-effort -- a half-wiped,
/// inconsistent state (e.g. channels gone but old messages still present)
/// is worse than leaving everything as it was and failing loudly.
fn wipe_superseded_state(data_dir: &Path, config_dir: &Path) -> anyhow::Result<()> {
    for path in paths_to_wipe_for_pairing(data_dir, config_dir) {
        remove_path(&path).with_context(|| format!("failed to remove {}", path.display()))?;
    }
    Ok(())
}

/// Wipes the local state a confirmed live pairing redeem (see
/// `app::AppState::restart_with_ticket`) would otherwise conflict with,
/// then restarts this exact binary with `--pair <ticket>` so the actual
/// redeem goes through the normal, already-tested startup bootstrap path
/// (`pairing::bootstrap`) instead of a separate, independently-risky live
/// code path -- see `pairing.rs`'s module doc comment for why pairing
/// deliberately never tries to do this in place. Only ever called after
/// the TUI has already shut down cleanly (`Net::shutdown`, `ratatui::
/// restore`) -- see `main`'s call site.
///
/// On success this never returns: `exec_pair` replaces the process image
/// outright on unix. The only way this returns `Err` is a wipe failure
/// (nothing after it is touched) or the restart itself failing
/// (vanishingly rare -- e.g. the binary was removed out from under
/// itself); either way the ticket is still unredeemed and valid, so the
/// error names the exact manual command that still finishes pairing.
fn restart_for_pairing(dirs: &directories::ProjectDirs, ticket: &str) -> anyhow::Result<()> {
    wipe_superseded_state(dirs.data_dir(), dirs.config_dir())?;

    let exe = std::env::current_exe().context("failed to locate the current executable")?;
    let err = exec_pair(&exe, ticket);
    Err(err).with_context(|| {
        format!(
            "failed to restart automatically -- local state was already wiped; \
             run `{} --pair {ticket}` manually to finish pairing",
            exe.display()
        )
    })
}

/// Replaces this process outright with `<exe> --pair <ticket>` -- see
/// `restart_for_pairing`. Never returns on success.
#[cfg(unix)]
fn exec_pair(exe: &Path, ticket: &str) -> anyhow::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(exe)
        .arg("--pair")
        .arg(ticket)
        .exec()
        .into()
}

/// Non-unix fallback for `exec_pair`: there's no process-image-replacing
/// exec here, so spawn the new process and exit this one once it's
/// launched.
#[cfg(not(unix))]
fn exec_pair(exe: &Path, ticket: &str) -> anyhow::Error {
    match std::process::Command::new(exe)
        .arg("--pair")
        .arg(ticket)
        .spawn()
    {
        Ok(_) => std::process::exit(0),
        Err(err) => err.into(),
    }
}

/// Resolves this app's OS-specific project directories, shared by logging
/// and identity persistence.
fn project_dirs() -> anyhow::Result<directories::ProjectDirs> {
    directories::ProjectDirs::from("dev", "leyline", "leyline")
        .ok_or_else(|| anyhow::anyhow!("could not determine a project directory"))
}

/// Logs go to a file under the OS data dir instead of stdout, since stdout
/// is the alternate-screen TUI once the terminal is initialized.
fn init_logging(dirs: &directories::ProjectDirs) -> anyhow::Result<()> {
    std::fs::create_dir_all(dirs.data_dir())?;
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dirs.data_dir().join("leyline.log"))?;

    // Respect `RUST_LOG` if set; otherwise default to a level that shows
    // our own diagnostics and iroh-gossip's connection lifecycle (dialing,
    // joins) without iroh's very chatty low-level QUIC/relay tracing.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        tracing_subscriber::EnvFilter::new("info,leyline=debug,iroh_gossip=debug")
    });

    tracing_subscriber::fmt()
        .with_writer(Mutex::new(log_file))
        .with_ansi(false)
        .with_env_filter(filter)
        .init();

    Ok(())
}

/// Resources `run` needs besides the terminal, grouped into one struct so
/// its signature doesn't grow a parameter per resource.
struct Session {
    net: Net,
    net_rx: mpsc::Receiver<NetEvent>,
    replay_tx: mpsc::Sender<NetEvent>,
    joined_channels: Vec<String>,
    active_channel: String,
    store: MessageStore,
    backfill: BackfillStore,
    registry: ChannelRegistry,
    contacts: Contacts,
    last_seen: LastSeen,
    channel_renames: ChannelRenames,
    /// Which channels have auto-save on -- see `autosave`. Seeded into
    /// `AppState` (`load_autosave`) at startup; written when `/autosave`
    /// switches a channel and when one is left.
    autosave: AutoSave,
    autosave_limit: autosave::AutoSaveLimit,
    /// Enabled one-way outboxes and their durable de-duplication records.
    outbox: Outbox,
    /// Enabled receive-only channel-folder mirrors.
    sync_settings: SyncSettings,
    sync_activity: SyncActivity,
    dm_registry: DmRegistry,
    direct_registry: DirectRegistry,
    settings: Settings,
    forgotten_devices: ForgottenDevices,
    /// Where `/save` put each channel's files -- see `saved_files`. Written
    /// when a save finishes (`NetEvent::FileSaved`), read when the files
    /// overlay opens (`InputAction::CheckSavedFiles`).
    saved_files: SavedFiles,
    shared_files: SharedFiles,
    /// Where received files go -- see `files::files_root`. Seeded into
    /// `AppState` (`load_files_root`) at startup, which holds it from then
    /// on: `/folder <path>` replaces it there.
    files_root: PathBuf,
    /// How big a file may be to share -- see `files::size_limit`. Seeded
    /// into `AppState` (`load_size_limit`) at startup, which holds it from
    /// then on: `/maxsize` replaces it there.
    size_limit: files::SizeLimit,
    /// leyline's own data directory (`dirs.data_dir()`), where the file
    /// picker's remembered folder (`picker::remember_folder`), the files
    /// root (`files::set_files_root`) and the size limit
    /// (`files::set_size_limit`) are kept.
    data_dir: PathBuf,
}

/// A `/msg`-initiated DM channel creation in flight: who it's for, and any
/// message text to send once it's ready -- stashed when
/// `app::InputAction::StartDm` is handled, and consumed once the
/// corresponding `net::NetEvent::Joined` confirms the channel actually
/// subscribed (see `run`'s main select loop). Keyed by channel name in the
/// `pending_dms` map, since that's the only handle `Net::join` gives back
/// to us -- a freshly created channel has no ticket to carry a peer id
/// through the way `net::NetEvent::DmJoined` does for the recipient side.
struct PendingDm {
    peer: [u8; 32],
    text: Option<String>,
}

enum SyncTrashOutcome {
    Listed {
        channel: String,
        result: Result<Vec<sync_apply::TrashedFile>, String>,
    },
    Restored {
        channel: String,
        path: String,
        result: Result<sync_apply::Restore, String>,
    },
    Purged {
        channel: String,
        result: Result<usize, String>,
    },
}

enum SyncConflictOutcome {
    Resolved {
        channel: String,
        conflict: SyncConflict,
        result: Result<(), String>,
    },
}

/// How often each joined channel with any recorded history re-announces its
/// current root to the whole channel (see `net::Net::announce`), on top of
/// the immediate re-announce whenever a channel gains a gossip neighbor.
/// Much coarser than `tick` below, since this goes out over the network --
/// but still frequent enough that a peer only ever bootstrapped through one
/// specific channel member still hears from every other member within one
/// interval, not only whoever it happens to be a direct gossip neighbor of
/// (see concept.md's "Persistence & history backfill" section).
const HISTORY_ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30);
const PAUSED_SYNC_EVENT_LIMIT: usize = 256;

/// Announces `channel`'s current history root to the whole channel, if
/// we've recorded any messages for it yet (see `BackfillStore::current_root`
/// and `net::Net::announce`) -- a no-op otherwise, since there's nothing to
/// offer. Shared by the per-neighbor fast path and the periodic heartbeat in
/// The "snappy async event loop": crossterm's async `EventStream` and the
/// network's `NetEvent`s merged via `tokio::select!`, redrawing only when
/// something actually changed -- see concept.md's "snappy async event
/// loop". Returns the ticket from a confirmed live pairing redeem, if the
/// loop ended that way (see `app::AppState::restart_with_ticket`) rather
/// than a plain quit, for `main` to act on once the terminal is restored.
async fn run(
    terminal: &mut ratatui::DefaultTerminal,
    session: Session,
) -> anyhow::Result<Option<String>> {
    let Session {
        net,
        mut net_rx,
        replay_tx,
        joined_channels,
        active_channel,
        store,
        backfill,
        mut registry,
        mut contacts,
        mut last_seen,
        mut channel_renames,
        mut autosave,
        autosave_limit,
        mut outbox,
        mut sync_settings,
        mut sync_activity,
        mut dm_registry,
        mut direct_registry,
        mut settings,
        mut forgotten_devices,
        mut saved_files,
        mut shared_files,
        files_root,
        size_limit,
        data_dir,
    } = session;

    let mut app = AppState::new(net.our_id, joined_channels.clone(), &active_channel);
    app.load_self_user_id(net.our_user_id());
    app.load_contacts(contacts.all());
    app.load_last_seen(last_seen.snapshot());
    app.load_renames(channel_renames.all());
    app.load_autosave(autosave.channels());
    app.load_autosave_limits(autosave.limits());
    app.load_autosave_limit(autosave_limit);
    app.load_outbox(outbox.channels());
    app.load_sync(sync_settings.configs());
    app.load_settings(settings.bell_enabled(), settings.nickname());
    app.load_dm_registry(dm_registry.all());
    app.load_forgotten_devices(forgotten_devices.all());
    app.load_browse_dir(start_folder(&data_dir));
    app.load_files_root(files_root.clone());
    app.load_size_limit(size_limit);
    for name in &joined_channels {
        // Ensures a brand-new channel ("general" on a first run, or a
        // `--join` ticket's channel) lands in the registry, so it's
        // rejoined automatically next time -- see channel_registry.rs.
        // A no-op for channels the registry already knew about. The
        // secret comes from `Net`, which already resolved (or generated)
        // the right one for each joined channel in `Net::start`.
        let secret = net
            .secret_for(name)
            .expect("just-joined channel must have a recorded secret");
        if let Err(err) = registry.record_channel(name, secret) {
            warn!(channel = %name, "failed to persist joined channel: {err}");
        } else if let Some(docs) = net.docs_for(name)
            && let Err(err) = registry.record_docs(name, docs)
        {
            warn!(channel = %name, "failed to persist channel index capability: {err}");
        }
        match store.load(name) {
            Ok(history) => {
                if let Err(err) = shared_files.record_messages(name, &history) {
                    warn!(channel = %name, "failed to index shared file history: {err}");
                }
                if let Err(err) = backfill.record_messages(name, &history).await {
                    warn!(channel = %name, "failed to seed history backfill: {err}");
                }
                app.load_history(name, history);
            }
            Err(err) => warn!(channel = %name, "failed to load message history: {err}"),
        }
    }

    // `DEVICE_SYNC_CHANNEL` is deliberately excluded from `joined_channels`
    // (see `net::Net::start`), since it must never become a visible tab --
    // but it still needs its own registry entry so `PeerAddressLearned`
    // events for it (handled in the match below, the same as for any other
    // channel) can accumulate bootstrap peers for a faster reconnect next
    // restart, exactly like every ordinary channel already does.
    if let Some(secret) = net.secret_for(DEVICE_SYNC_CHANNEL)
        && let Err(err) = registry.record_channel(DEVICE_SYNC_CHANNEL, secret)
    {
        warn!("failed to persist device-sync channel: {err}");
    }

    // Restores every known point-to-point conversation (see
    // direct_registry.rs): starts trying to (re)connect immediately
    // (`Net::open_direct`), recreates its tab in the background so it
    // doesn't steal focus from `active_channel`, and reloads its
    // persisted history -- mirroring the gossip-channel startup loop
    // above, so a restart behaves the same way for either kind of
    // channel. No `backfill` seeding here: that store only matters for
    // gossip's history-sync flow, which point-to-point channels never use.
    for peer in direct_registry.all() {
        let docs = direct_registry.docs_for(peer);
        let channel = net.open_direct(peer);
        net.sync_direct(peer, docs);
        if let Some(folder) = direct_registry.folder_for(peer) {
            app.load_direct_folder(channel.clone(), folder);
        }
        if app.background_direct_channel(channel.clone(), peer) {
            match store.load(&channel) {
                Ok(history) => {
                    if let Err(err) = shared_files.record_messages(&channel, &history) {
                        warn!(%channel, "failed to index shared file history: {err}");
                    }
                    app.load_history(&channel, history);
                }
                Err(err) => warn!(%channel, "failed to load message history: {err}"),
            }
        }
    }
    app.load_shared_files(shared_files.snapshot());

    // `None` if no clipboard is available (e.g. a headless SSH session) --
    // `/invite` still prints the ticket either way, this just skips the
    // auto-copy. Held for the whole session rather than recreated per
    // `/invite`, since a dropped `Clipboard` can lose its contents on
    // X11/Wayland if nothing else has claimed ownership yet.
    let mut clipboard = match Clipboard::new() {
        Ok(clipboard) => Some(clipboard),
        Err(err) => {
            debug!("clipboard unavailable, /invite won't auto-copy: {err}");
            None
        }
    };

    // `/msg`-initiated DM channel creations awaiting their `NetEvent::Joined`
    // confirmation, keyed by the freshly chosen channel name -- see
    // `PendingDm` and `InputAction::StartDm`'s handling below.
    let mut pending_dms: HashMap<String, PendingDm> = HashMap::new();
    let mut paused_sync_events: HashMap<String, VecDeque<NetEvent>> = HashMap::new();

    let mut term_events = TermEvents::new();
    let mut tick = interval(Duration::from_millis(250));
    let mut history_heartbeat = interval(HISTORY_ANNOUNCE_INTERVAL);
    // Reports for `/search`'s background on-disk scans (see search.rs and
    // `InputAction::Search`) -- kept separate from `net_rx`/`NetEvent`
    // since this isn't network activity, following the same "async work
    // reports into the select loop" shape.
    let (search_tx, mut search_rx) = mpsc::channel::<SearchOutcome>(8);
    // Likewise for the file picker's folder reads (see `InputAction::
    // ReadFolder`), which are filesystem work, not network activity.
    let (browse_tx, mut browse_rx) = mpsc::channel::<FolderListing>(8);
    let (files_tx, mut files_rx) = mpsc::channel::<FilesListing>(8);
    let (trash_tx, mut trash_rx) = mpsc::channel::<SyncTrashOutcome>(8);
    let (conflict_tx, mut conflict_rx) = mpsc::channel::<SyncConflictOutcome>(8);
    // Outbox directory reads stay off the event loop, exactly like the file
    // picker. One scan at a time prevents a slow network volume from piling
    // up blocking jobs.
    let (outbox_tx, mut outbox_rx) =
        mpsc::channel::<Vec<(String, Result<Vec<outbox::Candidate>, String>)>>(1);
    let mut outbox_watcher = OutboxWatcher::default();
    let mut outbox_scan_in_flight = false;
    let mut outbox_tick = interval(Duration::from_secs(1));
    let (sync_publish_tx, mut sync_publish_rx) =
        mpsc::channel::<Vec<(String, Result<Vec<sync_publish::Candidate>, String>)>>(1);
    let mut sync_publish_watcher = sync_publish::Watcher::default();
    let mut sync_publish_scan_in_flight = false;
    let mut sync_publish_tick = interval(Duration::from_secs(1));
    let mut dirty = true;
    let mut mouse_reporting = MouseReporting::new(std::io::stdout());
    let mut mouse_position = None;

    while !app.should_quit {
        mouse_reporting.update(app.show_hints)?;
        if dirty {
            let size = terminal.size()?;
            let area = ratatui::layout::Rect::new(0, 0, size.width, size.height);
            let target = mouse_position
                .and_then(|(column, row)| ui::command_hint_at(area, &app, column, row));
            app.set_hint_hover(target);
            let peer =
                mouse_position.and_then(|(column, row)| ui::peer_at(area, &app, column, row));
            app.set_peer_hover(peer);
            terminal.draw(|frame| ui::render(frame, &app))?;
            dirty = false;
        }

        tokio::select! {
            Some(event) = term_events.next() => {
                match event? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        match app.handle_key(key) {
                            Some(InputAction::Send(channel, message)) => {
                                if let Err(err) = store.append(&channel, &message) {
                                    warn!(%channel, "failed to persist sent message: {err}");
                                }
                                if app.direct_peer_for(&channel).is_none()
                                    && let Err(err) = backfill.record_message(&channel, &message).await
                                {
                                    warn!(%channel, "failed to record sent message for backfill: {err}");
                                }
                                route_send(&net, &app, &channel, message);
                            }
                            Some(InputAction::OpenDirect { peer, text }) => {
                                let channel = net.open_direct(peer);
                                if app.activate_direct_channel(channel.clone(), peer) {
                                    if let Err(err) =
                                        record_direct_peer(&mut direct_registry, &mut app, peer)
                                    {
                                        warn!(%channel, "failed to persist direct peer registry entry: {err}");
                                    }
                                    match store.load(&channel) {
                                        Ok(history) => app.load_history(&channel, history),
                                        Err(err) => warn!(%channel, "failed to load message history: {err}"),
                                    }
                                }
                                net.sync_direct(peer, direct_registry.docs_for(peer));
                                if let Some(text) = text {
                                    let message = app.compose_own_message(&text);
                                    net.send_direct(peer, message.clone());
                                    if let Err(err) = store.append(&channel, &message) {
                                        warn!(%channel, "failed to persist sent message: {err}");
                                    }
                                    app.record_sent_message(&channel, message);
                                }
                            }
                            Some(InputAction::SetOutbox { channel, on }) => {
                                let folder = app.folder_dir(&channel).join("send");
                                let result = if on {
                                    outbox.enable(&channel, &folder)
                                } else {
                                    outbox.disable(&channel)
                                };
                                match result {
                                    Ok(()) => {
                                        app.set_outbox(channel.clone(), on);
                                        if on {
                                            app.push_system(format!(
                                                "outbox is on for {} -- new files in {} will be shared once they stop changing",
                                                app.channel_heading(&channel),
                                                folder.display()
                                            ));
                                        } else {
                                            app.push_system(format!(
                                                "outbox is off for {}",
                                                app.channel_heading(&channel)
                                            ));
                                        }
                                    }
                                    Err(err) => app.push_system(format!(
                                        "couldn't turn outbox {} for {}: {err}",
                                        if on { "on" } else { "off" },
                                        app.channel_heading(&channel)
                                    )),
                                }
                            }
                            Some(InputAction::SetSync { channel, config }) => {
                                if let Err(err) = sync_settings.set(&channel, config.clone()) {
                                    warn!(%channel, "failed to persist sync setting: {err}");
                                    app.push_system(format!(
                                        "couldn't remember that sync is {}: {err}",
                                        config.label()
                                    ));
                                }
                            }
                            Some(InputAction::ListSyncTrash(channel)) => {
                                let root = app.folder_dir(&channel);
                                let tx = trash_tx.clone();
                                tokio::task::spawn_blocking(move || {
                                    let result = sync_apply::list_trash(&root);
                                    let _ = tx.blocking_send(SyncTrashOutcome::Listed {
                                        channel,
                                        result,
                                    });
                                });
                            }
                            Some(InputAction::ListSyncActivity(channel)) => {
                                let entries = sync_activity.recent(&channel);
                                if entries.is_empty() {
                                    app.push_system_to(&channel, "no sync activity recorded yet");
                                } else {
                                    app.push_system_to(&channel, format!("recent sync activity ({}):", entries.len()));
                                    for entry in entries {
                                        app.push_system_to(&channel, format!(
                                            "{} {}: {} ({})",
                                            entry.at_unix_ms, entry.action, entry.path, entry.outcome
                                        ));
                                    }
                                }
                            }
                            Some(InputAction::RestoreSyncTrash { channel, path }) => {
                                let root = app.folder_dir(&channel);
                                let tx = trash_tx.clone();
                                tokio::task::spawn_blocking(move || {
                                    let result = sync_apply::restore(&root, &path);
                                    let _ = tx.blocking_send(SyncTrashOutcome::Restored {
                                        channel,
                                        path,
                                        result,
                                    });
                                });
                            }
                            Some(InputAction::PurgeSyncTrash(channel)) => {
                                let root = app.folder_dir(&channel);
                                let tx = trash_tx.clone();
                                tokio::task::spawn_blocking(move || {
                                    let result = sync_apply::purge_trash(&root);
                                    let _ = tx.blocking_send(SyncTrashOutcome::Purged {
                                        channel,
                                        result,
                                    });
                                });
                            }
                            Some(InputAction::ResolveSyncConflict {
                                channel,
                                conflict,
                                resolution,
                            }) => {
                                let root = app.folder_dir(&channel);
                                let tx = conflict_tx.clone();
                                tokio::task::spawn_blocking(move || {
                                    let resolution = match resolution {
                                        ConflictResolution::Local => {
                                            sync_apply::ConflictResolution::Local
                                        }
                                        ConflictResolution::Remote => {
                                            sync_apply::ConflictResolution::Remote
                                        }
                                        ConflictResolution::Both => {
                                            sync_apply::ConflictResolution::Both
                                        }
                                    };
                                    let result = sync_apply::resolve_conflict(
                                        &root,
                                        &conflict.local_path,
                                        conflict.remote_copy_path.as_deref(),
                                        resolution,
                                    );
                                    let _ = tx.blocking_send(SyncConflictOutcome::Resolved {
                                        channel,
                                        conflict,
                                        result,
                                    });
                                });
                            }
                            Some(InputAction::Join(arg)) => {
                                if let Some(message) = net.join(arg) {
                                    app.push_system(message);
                                }
                            }
                            Some(InputAction::StartDm { peer, channel, text }) => {
                                pending_dms.insert(channel.clone(), PendingDm { peer, text });
                                if let Some(message) = net.join(channel) {
                                    app.push_system(message);
                                }
                            }
                            Some(InputAction::Invite(channel)) => {
                                let ticket =
                                    net.ticket_for(&channel, dm_registry.is_dm_channel(&channel));
                                let copied = clipboard.as_mut().is_some_and(|clipboard| {
                                    clipboard.set_text(ticket.clone()).is_ok()
                                });
                                let suffix = if copied { " (copied to clipboard)" } else { "" };
                                let heading = app.channel_heading(&channel);
                                app.push_system(format!("invite for {heading}: {ticket}{suffix}"));
                            }
                            Some(InputAction::Pair) => {
                                let snapshot = registry.snapshot();
                                let ticket = net.create_pairing_ticket(snapshot).encode_string();
                                let copied = clipboard.as_mut().is_some_and(|clipboard| {
                                    clipboard.set_text(ticket.clone()).is_ok()
                                });
                                let suffix = if copied { " (copied to clipboard)" } else { "" };
                                app.push_system(format!(
                                    "pairing ticket -- redeem on a brand-new device with `leyline --pair <ticket>`: {ticket}{suffix}"
                                ));
                            }
                            Some(InputAction::Alias(id, name)) => {
                                if let Err(err) = contacts.set(id, name) {
                                    warn!("failed to persist pet name: {err}");
                                }
                            }
                            Some(InputAction::Rename { channel, name }) => {
                                let persisted = match name {
                                    Some(name) => channel_renames.set(channel.clone(), name),
                                    None => channel_renames.forget(&channel),
                                };
                                if let Err(err) = persisted {
                                    warn!(%channel, "failed to persist channel rename: {err}");
                                }
                            }
                            Some(InputAction::ForgetDevice(id)) => {
                                if let Err(err) = forgotten_devices.forget(id) {
                                    warn!("failed to persist forgotten device: {err}");
                                }
                            }
                            Some(InputAction::Nick(name)) => {
                                if let Err(err) = settings.set_nickname(name.clone()) {
                                    warn!("failed to persist nickname: {err}");
                                }
                                net.set_nickname(name);
                            }
                            Some(InputAction::Leave(channel)) => {
                                // Auto-save is forgotten first, so that if
                                // anything below fails -- or the process dies
                                // midway -- a channel is left with auto-save
                                // off, never a setting left on a channel that
                                // is gone and whose name a different room
                                // could be given later.
                                if let Err(err) = autosave.set(&channel, false) {
                                    warn!(%channel, "failed to forget the auto-save setting: {err}");
                                }
                                if let Err(err) = outbox.disable(&channel) {
                                    warn!(%channel, "failed to forget the outbox setting: {err}");
                                }
                                if let Err(err) = sync_settings.forget(&channel) {
                                    warn!(%channel, "failed to forget the sync setting: {err}");
                                }
                                if let Err(err) = sync_activity.forget(&channel) {
                                    warn!(%channel, "failed to forget sync activity: {err}");
                                }
                                match app.direct_peer_for(&channel) {
                                    Some(peer) => {
                                        net.close_direct(peer);
                                        if let Err(err) = direct_registry.forget(peer) {
                                            warn!(%channel, "failed to remove direct peer registry entry: {err}");
                                        }
                                    }
                                    None => net.leave(&channel),
                                }
                                if let Err(err) = registry.forget_channel(&channel) {
                                    warn!(%channel, "failed to remove channel from registry: {err}");
                                }
                                if let Err(err) = dm_registry.forget_channel(&channel) {
                                    warn!(%channel, "failed to remove DM registry entry: {err}");
                                }
                                if let Err(err) = channel_renames.forget(&channel) {
                                    warn!(%channel, "failed to remove channel rename: {err}");
                                }
                                if let Err(err) = last_seen.forget_channel(&channel) {
                                    warn!(%channel, "failed to forget peer last-seen data: {err}");
                                }
                                if let Err(err) = store.delete(&channel) {
                                    warn!(%channel, "failed to delete channel history: {err}");
                                }
                                if let Err(err) = shared_files.forget_channel(&channel) {
                                    warn!(%channel, "failed to forget shared file metadata: {err}");
                                }
                                backfill.forget_channel(&channel);
                                app.remove_channel(&channel);
                            }
                            Some(InputAction::Search { channel, term }) => {
                                // A large on-disk log can't be assumed
                                // cheap enough for the async event loop --
                                // run it on tokio's blocking pool instead,
                                // per search.rs's doc comment.
                                let store = store.clone();
                                let search_tx = search_tx.clone();
                                tokio::task::spawn_blocking(move || {
                                    let outcome = run_on_disk_scan(&store, &channel, &term);
                                    let _ = search_tx.blocking_send(outcome);
                                });
                            }
                            Some(InputAction::SendFile { channel, path }) => {
                                if let Some(message) =
                                    net.send_file(channel, path, app.size_limit())
                                {
                                    app.push_system(message);
                                }
                            }
                            // `Enter` with attachments staged (see
                            // `app::Channel::staged`): importing and
                            // announcing them happens in the background,
                            // each finished file coming back as
                            // `FileReady` below, just like a `/send`.
                            Some(InputAction::SendStaged { channel, items, caption }) => {
                                net.send_staged(channel, items, caption);
                            }
                            Some(InputAction::SaveFile(request)) => {
                                start_save(&net, &mut app, request);
                            }
                            // `/autosave on|off`: already applied to `app`, so
                            // only remembering it is left. A write that fails
                            // leaves it in force for this session -- and says
                            // so, with what that means for the next: an `off`
                            // that didn't stick is `on` again after a restart.
                            Some(InputAction::SetAutoSave { channel, on }) => {
                                if let Err(err) = autosave.set(&channel, on) {
                                    warn!(%channel, "failed to persist the auto-save setting: {err}");
                                    app.push_system(if on {
                                        format!("couldn't remember that auto-save is on: {err} -- it holds until you quit")
                                    } else {
                                        format!("couldn't remember that auto-save is off: {err} -- it will be on again after a restart")
                                    });
                                }
                            }
                            Some(InputAction::SetChannelAutoSaveLimit { channel, limit }) => {
                                if let Err(err) = autosave.set_channel_limit(&channel, limit) {
                                    app.push_system(format!("couldn't remember the channel auto-save limit: {err}"));
                                }
                            }
                            // `/folder <path>`: only the filesystem can say
                            // whether the path will do, so it is made and
                            // remembered here, and `app` told once it took.
                            Some(InputAction::SetFilesRoot(path)) => {
                                match set_files_root(&data_dir, &path) {
                                    Ok(root) => {
                                        app.push_system(format!(
                                            "files will now be saved under {} (files saved before stay where they are)",
                                            root.display()
                                        ));
                                        app.load_files_root(root);
                                    }
                                    Err(message) => app.push_system(message),
                                }
                            }
                            // `/maxsize <size>`: already applied to `app`,
                            // so only remembering it is left. A write that
                            // fails still leaves the limit in force for this
                            // session -- and says so.
                            Some(InputAction::SetSizeLimit(limit)) => {
                                if let Err(message) = set_size_limit(&data_dir, limit) {
                                    app.push_system(message);
                                }
                            }
                            Some(InputAction::ReadFiles {
                                request,
                                channel,
                                dir,
                            }) => {
                                app.apply_saved_files(&channel, saved_files.present_in(&channel));
                                let files_tx = files_tx.clone();
                                tokio::task::spawn_blocking(move || {
                                    let files = sync_publish::scan(channel.clone(), dir).map(
                                        |candidates| {
                                            candidates
                                                .into_iter()
                                                .map(|candidate| LocalFile {
                                                    path: candidate.path,
                                                    relative: candidate.relative,
                                                    size: candidate.size,
                                                    modified_unix_ms: candidate
                                                        .modified
                                                        .duration_since(
                                                            std::time::SystemTime::UNIX_EPOCH,
                                                        )
                                                        .unwrap_or_default()
                                                        .as_millis()
                                                        .try_into()
                                                        .unwrap_or(u64::MAX),
                                                })
                                                .collect()
                                        },
                                    );
                                    let _ = files_tx.blocking_send(FilesListing {
                                        request,
                                        channel,
                                        files,
                                    });
                                });
                            }
                            // `Ctrl+O` in the files overlay. The copy may have
                            // been deleted since the overlay looked, which
                            // the launcher would only report by failing out
                            // of sight, so look first. The launcher is
                            // started, not waited for.
                            Some(InputAction::OpenFile(path)) => {
                                if !path.is_file() {
                                    app.push_system(format!(
                                        "can't open {}: it has been moved or deleted",
                                        path.display()
                                    ));
                                } else if let Err(err) = open_in_default_app(&path) {
                                    app.push_system(format!(
                                        "can't open {}: {err}",
                                        path.display()
                                    ));
                                }
                            }
                            Some(InputAction::RevealFile(path)) => {
                                if !path.is_file() {
                                    app.push_system(format!(
                                        "can't reveal {}: it has been moved or deleted",
                                        path.display()
                                    ));
                                } else if let Err(err) = reveal_in_file_manager(&path) {
                                    app.push_system(format!(
                                        "can't reveal {}: {err}",
                                        path.display()
                                    ));
                                }
                            }
                            Some(InputAction::EditFile(path)) => {
                                if !path.is_file() {
                                    app.push_system(format!(
                                        "can't edit {}: it has been moved or deleted",
                                        path.display()
                                    ));
                                } else if let Err(err) = edit_in_text_editor(&path) {
                                    app.push_system(format!(
                                        "can't edit {}: {err}",
                                        path.display()
                                    ));
                                }
                            }
                            Some(InputAction::Paste(channel)) => {
                                net.paste(channel);
                            }
                            Some(InputAction::Bell(enabled)) => {
                                if let Err(err) = settings.set_bell_enabled(enabled) {
                                    warn!("failed to persist bell setting: {err}");
                                }
                            }
                            // The file picker wants a folder listed. A huge or
                            // slow one -- a network mount -- can't be assumed
                            // cheap, so it's read on tokio's blocking pool and
                            // answers through `browse_rx` below: input stays
                            // live while it's pending.
                            Some(InputAction::ReadFolder { request, dir }) => {
                                let browse_tx = browse_tx.clone();
                                tokio::task::spawn_blocking(move || {
                                    let result = read_folder(&dir);
                                    let _ = browse_tx.blocking_send(FolderListing { request, result });
                                });
                            }
                            // Files chosen in the picker: staged for review
                            // exactly like a drop, and the folder they came
                            // from remembered for the next time it opens.
                            Some(InputAction::StageFiles { channel, paths, folder }) => {
                                stage_files(&mut app, &channel, &paths);
                                if let Err(err) = remember_folder(&data_dir, &folder) {
                                    warn!("failed to remember the picker's folder: {err}");
                                }
                            }
                            None => {}
                        }
                        dirty = true;
                    }
                    Event::Mouse(mouse) => {
                        mouse_position = Some((mouse.column, mouse.row));
                        match mouse.kind {
                            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                                let code = if mouse.kind == MouseEventKind::ScrollUp { KeyCode::Up } else { KeyCode::Down };
                                app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
                                dirty = true;
                            }
                            MouseEventKind::Moved => {
                                let size = terminal.size()?;
                                let area = ratatui::layout::Rect::new(0, 0, size.width, size.height);
                                dirty |= app.set_hint_hover(ui::command_hint_at(area, &app, mouse.column, mouse.row));
                                dirty |= app.set_peer_hover(ui::peer_at(area, &app, mouse.column, mouse.row));
                            }
                            _ => {}
                        }
                    }
                    Event::FocusLost => {
                        mouse_position = None;
                        dirty |= app.set_hint_hover(None);
                        dirty |= app.set_peer_hover(None);
                    }
                    Event::Resize(_, _) => dirty = true,
                    Event::Paste(text) => {
                        handle_paste(&mut app, &text);
                        dirty = true;
                    }
                    _ => {}
                }
            }
            _ = outbox_tick.tick(), if !outbox_scan_in_flight => {
                let channels = outbox.channels();
                if !channels.is_empty() {
                    let folders: Vec<_> = channels
                        .into_iter()
                        .map(|channel| (app.folder_dir(&channel).join("send"), channel))
                        .collect();
                    let tx = outbox_tx.clone();
                    outbox_scan_in_flight = true;
                    tokio::task::spawn_blocking(move || {
                        let results = folders
                            .into_iter()
                            .map(|(folder, channel)| {
                                let result = outbox::scan(channel.clone(), folder);
                                (channel, result)
                            })
                            .collect();
                        let _ = tx.blocking_send(results);
                    });
                }
            }
            _ = sync_publish_tick.tick(), if !sync_publish_scan_in_flight => {
                let channels = sync_settings.publishing_channels();
                if !channels.is_empty() {
                    let folders: Vec<_> = channels
                        .into_iter()
                        .map(|channel| (app.folder_dir(&channel), channel))
                        .collect();
                    let tx = sync_publish_tx.clone();
                    sync_publish_scan_in_flight = true;
                    tokio::task::spawn_blocking(move || {
                        let results = folders
                            .into_iter()
                            .map(|(folder, channel)| {
                                let result = sync_publish::scan(channel.clone(), folder);
                                (channel, result)
                            })
                            .collect();
                        let _ = tx.blocking_send(results);
                    });
                }
            }
            Some(net_event) = net_rx.recv() => {
                // Peek (without consuming) for a channel gaining a gossip
                // neighbor, so we can also announce our history root, our
                // current nickname (if any), and our build version to it --
                // see backfill.rs, concept.md's "Persistence & history
                // backfill" section, and `Net::announce_nickname`/
                // `Net::announce_version`. `app.handle_net_event` below
                // still separately updates presence for this same event.
                if let NetEvent::PeerJoined(channel, _) = &net_event {
                    announce_history(&net, &backfill, channel);
                    net.announce_nickname(channel);
                    net.announce_version(channel);
                    net.announce_device(channel);
                    net.announce_docs(channel);
                }
                if let NetEvent::PeerJoined(channel, id) = &net_event {
                    let seen_unix_ms = message::now_unix_ms();
                    app.record_peer_seen(channel, *id, seen_unix_ms);
                    if let Err(err) = last_seen.record(channel, *id, seen_unix_ms) {
                        warn!(%channel, "failed to persist peer last-seen time: {err}");
                    }
                }

                // Also peek for a channel finishing a runtime `/join`, so
                // it's persisted to the channel registry -- see
                // channel_registry.rs -- and rejoined automatically on a
                // future restart. Channels joined at startup are already
                // in the registry (see the loop above), so this only
                // matters for new ones joined during this session. Covers
                // both an ordinary `Joined` and the DM-ticket recipient's
                // `DmJoined` (see `ticket::ChannelTicket::dm`) -- both
                // represent a channel finishing its join.
                let joined_name = match &net_event {
                    NetEvent::Joined(name) => Some(name.as_str()),
                    NetEvent::DmJoined { channel, .. } => Some(channel.as_str()),
                    NetEvent::ChannelSyncJoined(name) => Some(name.as_str()),
                    _ => None,
                };
                if let Some(name) = joined_name {
                    let secret = net
                        .secret_for(name)
                        .expect("just-joined channel must have a recorded secret");
                    let is_new = !registry.channel_names().iter().any(|known| known == name);
                    if let Err(err) = registry.record_channel(name, secret) {
                        warn!(channel = %name, "failed to persist joined channel: {err}");
                    } else if let Some(docs) = net.docs_for(name)
                        && let Err(err) = registry.record_docs(name, docs)
                    {
                        warn!(channel = %name, "failed to persist channel index capability: {err}");
                    } else if is_new && !matches!(&net_event, NetEvent::ChannelSyncJoined(_)) {
                        // Tell our other devices too, over the private
                        // device-sync channel (see `Net::
                        // announce_channel_joined`) -- but never for a
                        // `ChannelSyncJoined` itself, which is how we
                        // learned about a channel from a sibling device in
                        // the first place; broadcasting it right back would
                        // just echo between devices instead of propagating
                        // anything new.
                        net.announce_channel_joined(name);
                    }
                }

                // Peek for the recipient side of a `/msg`-initiated DM (a
                // ticket-join that decoded with `dm: true`) -- persists the
                // peer<->channel association `dm_registry.rs` remembers
                // across restarts; `app.handle_net_event`'s own `DmJoined`
                // arm records the same association in `AppState` itself.
                if let NetEvent::DmJoined { channel, peer } = &net_event
                    && let Err(err) = dm_registry.record(*peer, channel.clone())
                {
                    warn!(channel = %channel, "failed to persist DM registry entry: {err}");
                }

                // Peek for the *initiator* side of a `/msg`-initiated DM
                // finishing its join instead -- a plain `Joined`, since a
                // freshly created channel has no ticket to carry a peer id
                // through (see `Net::join`'s doc comment) -- correlated
                // back to who it was for via `pending_dms` (populated when
                // `InputAction::StartDm` was handled above). `AppState`
                // doesn't learn the association from `handle_net_event` in
                // this case (a plain `Joined` carries no peer), so it's
                // told directly (`record_dm_channel`) once persisted here;
                // the queued text and invite ticket are handled after the
                // match below, once the channel is actually active.
                let finished_dm: Option<(String, PendingDm)> = match &net_event {
                    NetEvent::Joined(name) => {
                        pending_dms.remove(name).map(|pending| (name.clone(), pending))
                    }
                    _ => None,
                };
                if let Some((name, pending)) = &finished_dm {
                    if let Err(err) = dm_registry.record(pending.peer, name.clone()) {
                        warn!(channel = %name, "failed to persist DM registry entry: {err}");
                    }
                    app.record_dm_channel(pending.peer, name.clone());
                }

                // Peek for a finished `/save`, to remember where the copy
                // went -- what the files overlay marks as saved, even
                // after a restart (see `saved_files`). `app.handle_net_event`
                // below still separately posts the notice, and marks the
                // file in an open overlay.
                if let NetEvent::FileSaved { channel, hash, path, .. } = &net_event
                    && let Err(err) = saved_files.record(channel, *hash, path)
                {
                    warn!(%channel, "failed to record a saved file: {err}");
                }

                match net_event {
                    NetEvent::Announce(channel, announce) => {
                        net.sync_history(channel, announce);
                    }
                    NetEvent::HistoryFetched(channel, messages) => {
                        let newly_accepted = app.merge_history(&channel, messages);
                        for message in &newly_accepted {
                            if let Err(err) = store.append(&channel, message) {
                                warn!(%channel, "failed to persist backfilled message: {err}");
                            }
                                record_shared_file(&mut shared_files, &mut app, &channel, message);
                        }
                        if !newly_accepted.is_empty()
                            && let Err(err) = backfill.record_messages(&channel, &newly_accepted).await
                        {
                            warn!(%channel, "failed to record backfilled messages: {err}");
                        }
                    }
                    NetEvent::PeerAddressLearned(channel, addr) => {
                        if let Err(err) = registry.record_peer(&channel, addr) {
                            warn!(%channel, "failed to persist known peer address: {err}");
                        }
                    }
                    // A sibling device announced joining a channel -- see
                    // `crate::message::ChannelSyncAnnounce`. Ignored
                    // outright if the sender is a locally forgotten device
                    // (see `crate::forgotten_devices` and `/forget-device`);
                    // otherwise handed to `Net::sync_channel`, which
                    // double-checks it actually arrived on
                    // `DEVICE_SYNC_CHANNEL` before acting on it. A
                    // successful join reports back as
                    // `NetEvent::ChannelSyncJoined`, handled like any other
                    // `Joined`-shaped event above and below.
                    NetEvent::ChannelSync(tag, announce) => {
                        if !forgotten_devices.is_forgotten(&announce.sender) {
                            net.sync_channel(&tag, announce);
                        }
                    }
                    NetEvent::ChannelDocs(channel, announce) => {
                        net.adopt_channel_docs(&channel, announce);
                    }
                    NetEvent::ChannelDocsAdopted(channel, docs) => {
                        if let Err(err) = registry.record_docs(&channel, docs) {
                            warn!(%channel, "failed to persist adopted channel index capability: {err}");
                        }
                        net.announce_docs(&channel);
                    }
                    NetEvent::RemoteIndex(remote) => {
                        if app.sync_paused(&remote.channel) {
                            queue_paused_sync_event(
                                &mut paused_sync_events,
                                remote.channel.clone(),
                                NetEvent::RemoteIndex(remote),
                            );
                        } else if app.sync_receives(&remote.channel)
                            && app.sync_allows_path(&remote.channel, &remote.path)
                        {
                            if remote.entry.size > files::DEFAULT_MAX_FILE_SIZE {
                                let _ = sync_activity.record(Activity { channel: remote.channel.clone(), path: remote.path.clone(), action: "download".into(), outcome: "skipped: too large".into(), at_unix_ms: message::now_unix_ms() });
                                app.push_system_to(&remote.channel, format!(
                                    "sync skipped {}: {} exceeds the {} sync limit (fixed, independent of /maxsize)",
                                    remote.path, files::human_size(remote.entry.size),
                                    files::human_size(files::DEFAULT_MAX_FILE_SIZE)
                                ));
                            } else {
                                let root = app.folder_dir(&remote.channel);
                                let relocation = saved_files
                                    .present_in(&remote.channel)
                                    .get(&remote.entry.content_hash)
                                    .cloned()
                                    .map(|source| {
                                        sync_apply::relocate(&root, &source, &remote.path)
                                            .map(|result| (source, result))
                                    });
                                match relocation {
                                    Some(Ok((source, sync_apply::Relocation::Moved(destination)))) => {
                                        if let Err(err) = saved_files.move_path(
                                            &remote.channel,
                                            &source,
                                            &destination,
                                        ) {
                                            warn!(%remote.channel, "failed to update relocated sync file: {err}");
                                        }
                                        record_remote_sync_file(
                                            &mut sync_publish_watcher,
                                            remote.channel,
                                            remote.path,
                                            destination,
                                        );
                                    }
                                    Some(Ok((_source, sync_apply::Relocation::AlreadyThere))) => {}
                                    Some(Ok((_, sync_apply::Relocation::Unavailable))) | None => match sync_apply::reserve(&root, &remote.path) {
                                    Ok(destination) => {
                                        if !destination.ends_with(&remote.path) {
                                            let local = root.join(&remote.path);
                                            let local_modified_unix_ms = std::fs::metadata(&local)
                                                .ok()
                                                .and_then(|metadata| metadata.modified().ok())
                                                .and_then(|modified| modified.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                                                .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX));
                                            let remote_copy_path = destination
                                                .strip_prefix(&root)
                                                .ok()
                                                .and_then(|path| path.to_str())
                                                .map(|path| path.replace(std::path::MAIN_SEPARATOR, "/"));
                                            app.sync_conflict(&remote.channel, SyncConflict {
                                                local_path: remote.path.clone(),
                                                remote_path: remote.path.clone(),
                                                remote_copy_path,
                                                local_modified_unix_ms,
                                                remote_modified_unix_ms: Some(remote.entry.modified_unix_ms),
                                                author: Some(remote.entry.author),
                                            });
                                            let _ = sync_activity.record(Activity { channel: remote.channel.clone(), path: remote.path.clone(), action: "download".into(), outcome: "conflict".into(), at_unix_ms: message::now_unix_ms() });
                                        }
                                        app.sync_pending(&remote.channel, remote.path.clone());
                                        net.save_synced_file(
                                            remote.channel,
                                            remote.entry.content_hash,
                                            remote.path,
                                            vec![remote.entry.author],
                                            destination,
                                            files::DEFAULT_MAX_FILE_SIZE,
                                        );
                                    }
                                    Err(reason) => app.push_system_to(
                                        &remote.channel,
                                        format!("{reason} -- check the folder's permissions and free disk space"),
                                    ),
                                    },
                                    Some(Err(reason)) => app.push_system_to(
                                        &remote.channel,
                                        format!("{reason} -- check the folder's permissions and free disk space"),
                                    ),
                                }
                            }
                        }
                    }
                    NetEvent::RemoteIndexDelete { channel, path } => {
                        if app.sync_paused(&channel) {
                            queue_paused_sync_event(
                                &mut paused_sync_events,
                                channel.clone(),
                                NetEvent::RemoteIndexDelete { channel, path },
                            );
                        } else if app.sync_receives(&channel)
                            && app.sync_allows_path(&channel, &path)
                        {
                            if !sync_publish_watcher.may_remove_remote(&channel, &path) {
                                let _ = sync_activity.record(Activity { channel: channel.clone(), path: path.clone(), action: "delete".into(), outcome: "conflict: local edit kept".into(), at_unix_ms: message::now_unix_ms() });
                                let root = app.folder_dir(&channel);
                                let local_modified_unix_ms = std::fs::metadata(root.join(&path))
                                    .ok()
                                    .and_then(|metadata| metadata.modified().ok())
                                    .and_then(|modified| {
                                        modified.duration_since(std::time::SystemTime::UNIX_EPOCH).ok()
                                    })
                                    .map(|duration| {
                                        duration.as_millis().try_into().unwrap_or(u64::MAX)
                                    });
                                app.sync_conflict(
                                    &channel,
                                    SyncConflict {
                                        local_path: path.clone(),
                                        remote_path: path.clone(),
                                        remote_copy_path: None,
                                        local_modified_unix_ms,
                                        remote_modified_unix_ms: None,
                                        author: None,
                                    },
                                );
                                app.push_system_to(
                                    &channel,
                                    format!("sync kept locally changed {path} after a remote delete"),
                                );
                            } else {
                                let root = app.folder_dir(&channel);
                                match sync_apply::trash(&root, &path) {
                                    Ok(sync_apply::Trash::Moved(trash)) => {
                                        let _ = sync_activity.record(Activity { channel: channel.clone(), path: path.clone(), action: "delete".into(), outcome: "moved to trash".into(), at_unix_ms: message::now_unix_ms() });
                                        let source = root.join(&path);
                                        if let Err(err) = saved_files.move_path(&channel, &source, &trash) {
                                            warn!(%channel, "failed to update trashed sync file: {err}");
                                        }
                                        sync_publish_watcher.removed_remote(&channel, &path);
                                        app.push_system_to(
                                            &channel,
                                            format!("sync moved {path} to {}", trash.display()),
                                        );
                                    }
                                    Ok(sync_apply::Trash::Missing) => {
                                        sync_publish_watcher.removed_remote(&channel, &path);
                                    }
                                    Err(reason) => app.push_system_to(
                                        &channel,
                                        format!("{reason} -- check the folder's permissions and free disk space"),
                                    ),
                                }
                            }
                        }
                    }
                    NetEvent::SyncFileSaved {
                        channel,
                        hash,
                        filename,
                        path,
                    } => {
                        let _ = sync_activity.record(Activity {
                            channel: channel.clone(),
                            path: filename.clone(),
                            action: "download".into(),
                            outcome: "applied".into(),
                            at_unix_ms: message::now_unix_ms(),
                        });
                        app.sync_succeeded(&channel, &filename, message::now_unix_ms());
                        let root = app.folder_dir(&channel);
                        match path.strip_prefix(&root).ok().and_then(|path| path.to_str()) {
                            Some(relative) => record_remote_sync_file(
                                &mut sync_publish_watcher,
                                channel.clone(),
                                relative.replace(std::path::MAIN_SEPARATOR, "/"),
                                path.clone(),
                            ),
                            None => warn!(path = %path.display(), "sync file completed outside its channel folder"),
                        }
                        if let Err(err) = saved_files.record(&channel, hash, &path) {
                            warn!(%channel, "failed to record synced file: {err}");
                        }
                        app.handle_net_event(NetEvent::FileSaved {
                            channel,
                            hash,
                            filename,
                            path,
                            mode: files::SaveMode::Automatic {
                                max_bytes: files::DEFAULT_MAX_FILE_SIZE,
                            },
                        });
                    }
                    // A peer reached us over a point-to-point connection
                    // for the first time this session (see `direct.rs`) --
                    // create (or confirm) its background tab, mirroring
                    // `ChannelSyncJoined`'s "don't steal focus" semantics,
                    // and load its persisted history the first time, the
                    // same way the startup restore loop above does.
                    NetEvent::DirectJoined { channel, peer } => {
                        if app.background_direct_channel(channel.clone(), peer) {
                            if let Err(err) =
                                record_direct_peer(&mut direct_registry, &mut app, peer)
                            {
                                warn!(%channel, "failed to persist direct peer registry entry: {err}");
                            }
                            match store.load(&channel) {
                                Ok(history) => app.load_history(&channel, history),
                                Err(err) => warn!(%channel, "failed to load message history: {err}"),
                            }
                        }
                        net.sync_direct(peer, direct_registry.docs_for(peer));
                    }
                    NetEvent::DirectDocs { peer, docs } => {
                        if peer >= net.our_id {
                            warn!("ignoring direct docs capability from non-owner");
                        } else if let Err(err) =
                            record_direct_peer(&mut direct_registry, &mut app, peer)
                        {
                            warn!("failed to persist direct peer registry entry: {err}");
                        } else if let Err(err) = direct_registry.record_docs(peer, docs.clone()) {
                            warn!("failed to persist direct docs capability: {err}");
                        } else {
                            net.sync_direct(peer, Some(docs));
                        }
                    }
                    NetEvent::DirectDocsReady { peer, docs } => {
                        if let Err(err) =
                            record_direct_peer(&mut direct_registry, &mut app, peer)
                        {
                            warn!("failed to persist direct peer registry entry: {err}");
                        } else if let Err(err) = direct_registry.record_docs(peer, docs) {
                            warn!("failed to persist direct docs capability: {err}");
                        }
                    }
                    // A `/send`ed file failed to import -- see
                    // `net::Net::send_file`. Logged (unlike most other
                    // system notices, which app.rs pushes on its own)
                    // since it's a user-triggered action worth a trace in
                    // leyline.log for later debugging.
                    NetEvent::FileSendFailed { path, error } => {
                        warn!(%path, "failed to send file: {error}");
                        app.push_system(format!("failed to send {path}: {error}"));
                    }
                    // A saved file failed to download or write to disk
                    // -- see `net::Net::save_file`. Logged for the same
                    // reason as `FileSendFailed` above; what is said, and
                    // where, depends on how the save came about
                    // (`AppState::save_failed`).
                    NetEvent::FileSaveFailed { channel, hash, filename, error, mode } => {
                        warn!(%filename, "failed to save file: {error}");
                        if app.sync_is_pending(&channel, &filename) {
                            app.sync_failed(&channel, filename.clone(), error.clone());
                        }
                        app.save_failed(&channel, hash, &filename, &error, mode);
                    }
                    // A `/send`ed file finished importing and is ready to
                    // broadcast -- see `net::Net::send_file`. Broadcasting
                    // here (rather than inside `send_file` itself) keeps
                    // every locally-authored message going out through the
                    // same `net.send` call, the same way `InputAction::Send`
                    // above does for typed text.
                    NetEvent::FileReady(channel, message) => {
                        route_send(&net, &app, &channel, message.clone());
                        if let Err(err) = store.append(&channel, &message) {
                            warn!(%channel, "failed to persist sent file: {err}");
                        }
                        record_shared_file(&mut shared_files, &mut app, &channel, &message);
                        if app.direct_peer_for(&channel).is_none()
                            && let Err(err) = backfill.record_message(&channel, &message).await
                        {
                            warn!(%channel, "failed to record sent file for backfill: {err}");
                        }
                        app.record_sent_message(&channel, message);
                    }
                    // `/paste` found real file(s) on the clipboard (e.g. a
                    // Finder/Explorer copy) -- see `net::Net::paste`.
                    // Staged for review, not shared: see `stage_files`.
                    NetEvent::ClipboardFiles(channel, paths) => {
                        stage_files(&mut app, &channel, &paths);
                    }
                    // `/paste` found rendered image pixels (e.g. a
                    // screenshot) and already re-encoded them as PNG --
                    // see `net::Net::paste`/`net::encode_png`. Staged
                    // like a file: see `stage_clipboard_image`.
                    NetEvent::ClipboardImage(channel, filename, bytes) => {
                        stage_clipboard_image(&mut app, &channel, filename, bytes);
                    }
                    // `/paste` found no file or image, but did find plain
                    // text -- share it exactly like a typed-and-sent
                    // message (see `NetEvent::FileReady` above for why
                    // broadcast/persist/backfill/display always happen
                    // together for a locally-authored message).
                    NetEvent::ClipboardText(channel, text) => {
                        let message = app.compose_own_message(&text);
                        route_send(&net, &app, &channel, message.clone());
                        if let Err(err) = store.append(&channel, &message) {
                            warn!(%channel, "failed to persist pasted message: {err}");
                        }
                        if app.direct_peer_for(&channel).is_none()
                            && let Err(err) = backfill.record_message(&channel, &message).await
                        {
                            warn!(%channel, "failed to record pasted message for backfill: {err}");
                        }
                        app.record_sent_message(&channel, message);
                    }
                    // `/paste` found nothing to share -- see
                    // `net::Net::paste`. Logged like `FileSendFailed`
                    // above, since it's a user-triggered action worth a
                    // trace in leyline.log for later debugging.
                    NetEvent::PasteFailed(error) => {
                        warn!("paste failed: {error}");
                        app.push_system(format!("paste failed: {error}"));
                    }
                    other => {
                        if let Some((channel, message)) = app.handle_net_event(other) {
                            if let Err(err) = store.append(&channel, &message) {
                                warn!(%channel, "failed to persist received message: {err}");
                            }
                            record_shared_file(&mut shared_files, &mut app, &channel, &message);
                            if app.direct_peer_for(&channel).is_none()
                                && let Err(err) = backfill.record_message(&channel, &message).await
                            {
                                warn!(%channel, "failed to record received message for backfill: {err}");
                            }
                            if app.bell_enabled {
                                ring_bell();
                            }
                            // A file that has just arrived live may be
                            // saved on its own, if its channel has
                            // auto-save on -- see `AppState::auto_save`. Only
                            // here: history loaded at startup and history
                            // backfilled from a peer never come through
                            // this arm, so nothing from before is fetched.
                            if let Some(request) = app.auto_save(&channel, &message, |hash| {
                                saved_files.is_saved(&channel, hash)
                            }) {
                                start_save(&net, &mut app, request);
                            }
                        }
                    }
                }

                // Now that a finished creator-side DM (see `finished_dm`
                // above) is actually active (`app.handle_net_event` just ran
                // for its plain `Joined`), send any text queued by `/msg`
                // and surface the invite ticket the other party still needs
                // out-of-band, mirroring `/invite`.
                if let Some((channel, pending)) = finished_dm {
                    let ticket = net.ticket_for(&channel, true);
                    let copied = clipboard
                        .as_mut()
                        .is_some_and(|clipboard| clipboard.set_text(ticket.clone()).is_ok());
                    let suffix = if copied { " (copied to clipboard)" } else { "" };
                    app.push_system(format!("started #{channel} -- invite: {ticket}{suffix}"));
                    if let Some(text) = pending.text {
                        let message = app.compose_own_message(&text);
                        net.send(&channel, message.clone());
                        if let Err(err) = store.append(&channel, &message) {
                            warn!(%channel, "failed to persist sent message: {err}");
                        }
                        if let Err(err) = backfill.record_message(&channel, &message).await {
                            warn!(%channel, "failed to record sent message for backfill: {err}");
                        }
                        app.record_sent_message(&channel, message);
                    }
                }
                dirty = true;
            }
            Some(outcome) = search_rx.recv() => {
                app.apply_search_outcome(outcome);
                dirty = true;
            }
            Some(listing) = browse_rx.recv() => {
                app.apply_folder_listing(listing);
                dirty = true;
            }
            Some(listing) = files_rx.recv() => {
                app.apply_files_listing(listing);
                dirty = true;
            }
            Some(outcome) = trash_rx.recv() => {
                report_sync_trash(&mut app, outcome);
                dirty = true;
            }
            Some(SyncConflictOutcome::Resolved { channel, conflict, result }) = conflict_rx.recv() => {
                match result {
                    Ok(()) => {
                        app.sync_conflict_resolved(&channel, &conflict);
                        app.push_system_to(&channel, format!("resolved sync conflict for {}", conflict.local_path));
                    }
                    Err(error) => app.push_system_to(
                        &channel,
                        format!("couldn't resolve sync conflict for {}: {error}", conflict.local_path),
                    ),
                }
                dirty = true;
            }
            Some(results) = outbox_rx.recv() => {
                outbox_scan_in_flight = false;
                let mut candidates = Vec::new();
                for (channel, result) in results {
                    match result {
                        Ok(found) => candidates.extend(found),
                        Err(error) => app.push_system_to(&channel, error),
                    }
                }
                for candidate in outbox_watcher.stable(candidates, &outbox) {
                    if let Err(err) = outbox.mark_seen(&candidate.channel, candidate.name.clone()) {
                        app.push_system_to(
                            &candidate.channel,
                            format!(
                                "couldn't remember outbox file {}: {err}",
                                candidate.path.display()
                            ),
                        );
                        continue;
                    }
                    if let Some(error) = net.send_file(
                        candidate.channel.clone(),
                        candidate.path.display().to_string(),
                        app.size_limit(),
                    ) {
                        app.push_system_to(
                            &candidate.channel,
                            format!("outbox skipped {}: {error}", candidate.name),
                        );
                    }
                }
                dirty = true;
            }
            Some(results) = sync_publish_rx.recv() => {
                sync_publish_scan_in_flight = false;
                let mut candidates = Vec::new();
                for (channel, result) in results {
                    match result {
                        Ok(found) => candidates.extend(found),
                        Err(error) => {
                            let _ = sync_activity.record(Activity { channel: channel.clone(), path: "<folder>".into(), action: "publish".into(), outcome: format!("failed: {error}"), at_unix_ms: message::now_unix_ms() });
                            app.push_system_to(&channel, error)
                        }
                    }
                }
                let changes = sync_publish_watcher.stable(candidates);
                for candidate in changes.iter().filter_map(|change| match change {
                    sync_publish::Change::Present(candidate) => Some(candidate),
                    sync_publish::Change::Absent { .. } => None,
                }) {
                    if !sync_settings.allows_path(&candidate.channel, &candidate.relative) {
                        continue;
                    }
                    if candidate.size > files::DEFAULT_MAX_FILE_SIZE {
                        let _ = sync_activity.record(Activity { channel: candidate.channel.clone(), path: candidate.relative.clone(), action: "publish".into(), outcome: "skipped: too large".into(), at_unix_ms: message::now_unix_ms() });
                        app.push_system_to(
                            &candidate.channel,
                            format!(
                                "sync skipped {}: {} exceeds the {} sync limit (fixed, independent of /maxsize)",
                                candidate.relative,
                                files::human_size(candidate.size),
                                files::human_size(files::DEFAULT_MAX_FILE_SIZE)
                            ),
                        );
                        continue;
                    }
                    let modified_unix_ms = candidate
                        .modified
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                        .try_into()
                        .unwrap_or(u64::MAX);
                    net.publish_index_file(
                        candidate.channel.clone(),
                        candidate.path.clone(),
                        candidate.relative.clone(),
                        candidate.size,
                        modified_unix_ms,
                    );
                    let _ = sync_activity.record(Activity { channel: candidate.channel.clone(), path: candidate.relative.clone(), action: "publish".into(), outcome: "queued".into(), at_unix_ms: message::now_unix_ms() });
                }
                for change in changes {
                    if let sync_publish::Change::Absent { channel, relative } = change
                        && sync_settings.allows_path(&channel, &relative)
                    {
                        net.publish_index_delete(channel.clone(), relative.clone());
                        let _ = sync_activity.record(Activity { channel, path: relative, action: "delete".into(), outcome: "queued".into(), at_unix_ms: message::now_unix_ms() });
                    }
                }
                dirty = true;
            }
            _ = tick.tick() => {
                replay_paused_sync_events(&mut paused_sync_events, &app, &replay_tx);
            }
            _ = history_heartbeat.tick() => {
                // Re-announce every joined channel's history root
                // periodically, not just on neighbor churn -- covers peers
                // we're connected to the same swarm alongside but never
                // became a direct gossip neighbor of (see
                // `announce_history` and concept.md's "Persistence &
                // history backfill" section).
                for channel in &app.channels {
                    announce_history(&net, &backfill, &channel.name);
                    net.announce_docs(&channel.name);
                }
            }
        }
    }

    if !wait_for_shutdown(Duration::from_secs(2), net.shutdown()).await? {
        warn!("timed out waiting for network shutdown; restoring the terminal anyway");
    }

    Ok(app.restart_with_ticket)
}
/// Waits for a graceful network shutdown without letting a stalled network
/// task hold the terminal in raw mode indefinitely. Returns `false` only
/// when the deadline elapsed; a completed shutdown error still propagates.
async fn wait_for_shutdown<F>(limit: Duration, shutdown: F) -> anyhow::Result<bool>
where
    F: Future<Output = anyhow::Result<()>>,
{
    match timeout(limit, shutdown).await {
        Ok(result) => {
            result?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}
fn report_sync_trash(app: &mut AppState, outcome: SyncTrashOutcome) {
    match outcome {
        SyncTrashOutcome::Listed {
            channel,
            result: Ok(files),
        } if files.is_empty() => app.push_system_to(
            &channel,
            format!(
                "sync trash for {} is empty -- recoverable remote deletes stay until /trash purge",
                app.channel_heading(&channel)
            ),
        ),
        SyncTrashOutcome::Listed {
            channel,
            result: Ok(files),
        } => {
            app.push_system_to(
                &channel,
                format!(
                    "sync trash for {} ({} recoverable file(s), kept until /trash purge):",
                    app.channel_heading(&channel),
                    files.len()
                ),
            );
            for file in files {
                app.push_system_to(
                    &channel,
                    format!("  {} ({})", file.relative, files::human_size(file.size)),
                );
            }
        }
        SyncTrashOutcome::Restored {
            channel,
            path,
            result: Ok(sync_apply::Restore::Restored(destination)),
        } => app.push_system_to(
            &channel,
            format!("restored {path} to {}", destination.display()),
        ),
        SyncTrashOutcome::Restored {
            channel,
            path,
            result: Ok(sync_apply::Restore::Missing),
        } => app.push_system_to(&channel, format!("no recoverable sync file named {path}")),
        SyncTrashOutcome::Purged {
            channel,
            result: Ok(0),
        } => app.push_system_to(&channel, "sync trash was already empty"),
        SyncTrashOutcome::Purged {
            channel,
            result: Ok(count),
        } => app.push_system_to(
            &channel,
            format!("permanently purged {count} recoverable sync file(s)"),
        ),
        SyncTrashOutcome::Listed {
            channel,
            result: Err(error),
        }
        | SyncTrashOutcome::Restored {
            channel,
            result: Err(error),
            ..
        }
        | SyncTrashOutcome::Purged {
            channel,
            result: Err(error),
        } => app.push_system_to(&channel, format!("sync trash failed: {error}")),
    }
}

fn record_direct_peer(
    registry: &mut DirectRegistry,
    app: &mut AppState,
    peer: [u8; 32],
) -> anyhow::Result<()> {
    if registry.record(peer)? {
        registry.record_folder(peer, files::direct_folder(&app.display_name(&peer), &peer))?;
    }
    if let Some(folder) = registry.folder_for(peer) {
        app.load_direct_folder(direct::channel_name(&peer), folder);
    }
    Ok(())
}

fn record_shared_file(
    shared_files: &mut SharedFiles,
    app: &mut AppState,
    channel: &str,
    message: &ChatMessage,
) {
    if let Err(err) = shared_files.record_message(channel, message) {
        warn!(%channel, "failed to index shared file: {err}");
        return;
    }
    app.load_shared_files(shared_files.snapshot());
}
fn record_remote_sync_file(
    watcher: &mut sync_publish::Watcher,
    channel: String,
    relative: String,
    path: PathBuf,
) {
    match std::fs::metadata(&path) {
        Ok(metadata) => watcher.record_remote(&sync_publish::Candidate {
            channel,
            path,
            relative,
            size: metadata.len(),
            modified: metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
        }),
        Err(err) => warn!(path = %path.display(), "failed to inspect synced file: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    #[test]
    fn mouse_reporting_is_disabled_when_its_owner_returns() {
        let mut bytes = Vec::new();
        {
            let mut mode = MouseReporting::new(&mut bytes);
            mode.update(true).unwrap();
            mode.update(true).unwrap(); // redraws must not re-enable modes
        }
        let mut expected = Vec::new();
        write_mouse_reporting(&mut expected, true).unwrap();
        write_mouse_reporting(&mut expected, false).unwrap();
        assert_eq!(bytes, expected);
        assert!(String::from_utf8(bytes).unwrap().ends_with("\x1b[?1000l"));
    }

    #[test]
    fn hiding_hints_disables_reporting_without_waiting_for_exit() {
        let mut bytes = Vec::new();
        {
            let mut mode = MouseReporting::new(&mut bytes);
            mode.update(true).unwrap();
            mode.update(false).unwrap();
        }
        let mut expected = Vec::new();
        write_mouse_reporting(&mut expected, true).unwrap();
        write_mouse_reporting(&mut expected, false).unwrap();
        assert_eq!(bytes, expected);
    }
    #[test]
    fn draining_terminal_events_consumes_ready_events_after_one_short_wait() {
        let mut ready = [true, true, false].into_iter();
        let mut timeouts = Vec::new();
        let mut reads = 0;

        let drained = drain_terminal_events(
            |timeout| {
                timeouts.push(timeout);
                Ok(ready.next().unwrap())
            },
            || {
                reads += 1;
                Ok(Event::FocusLost)
            },
        )
        .unwrap();

        assert_eq!(drained, 2);
        assert_eq!(reads, 2);
        assert_eq!(
            timeouts,
            vec![Duration::from_millis(10), Duration::ZERO, Duration::ZERO]
        );
    }

    #[test]
    fn draining_terminal_events_does_not_read_when_none_are_ready() {
        let mut reads = 0;

        let drained = drain_terminal_events(
            |_| Ok(false),
            || {
                reads += 1;
                Ok(Event::FocusLost)
            },
        )
        .unwrap();

        assert_eq!(drained, 0);
        assert_eq!(reads, 0);
    }

    #[tokio::test]
    async fn network_shutdown_wait_completes_when_the_shutdown_finishes() {
        assert!(
            wait_for_shutdown(Duration::ZERO, async { Ok(()) })
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn network_shutdown_wait_times_out_without_blocking_terminal_cleanup() {
        assert!(
            !wait_for_shutdown(Duration::ZERO, std::future::pending::<anyhow::Result<()>>())
                .await
                .unwrap()
        );
    }

    #[test]
    fn remove_path_deletes_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("some_file");
        std::fs::write(&path, b"data").unwrap();

        remove_path(&path).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn remove_path_deletes_an_existing_directory_and_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("some_dir");
        std::fs::create_dir_all(path.join("nested")).unwrap();
        std::fs::write(path.join("nested").join("file"), b"data").unwrap();

        remove_path(&path).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn remove_path_is_a_no_op_when_already_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("never_existed");

        remove_path(&path).unwrap();
    }

    #[test]
    fn wipe_superseded_state_removes_every_listed_path_and_nothing_else() {
        let data_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();
        for path in paths_to_wipe_for_pairing(data_dir.path(), config_dir.path()) {
            std::fs::write(&path, b"stale").unwrap();
        }
        // Stand in for `identity`/`leyline.log` -- never part of the wipe
        // list, even though `identity` lives in the very same directory
        // as `user_key` on some platforms (e.g. macOS, where `data_dir`
        // and `config_dir` coincide).
        let identity = config_dir.path().join("identity");
        let log = data_dir.path().join("leyline.log");
        std::fs::write(&identity, b"keep me").unwrap();
        std::fs::write(&log, b"keep me too").unwrap();

        wipe_superseded_state(data_dir.path(), config_dir.path()).unwrap();

        for path in paths_to_wipe_for_pairing(data_dir.path(), config_dir.path()) {
            assert!(
                !path.exists(),
                "{} should have been removed",
                path.display()
            );
        }
        assert!(identity.exists(), "identity must never be wiped by pairing");
        assert!(log.exists(), "leyline.log must never be wiped by pairing");
    }

    #[test]
    fn wipe_superseded_state_tolerates_paths_that_never_existed() {
        let data_dir = tempfile::tempdir().unwrap();
        let config_dir = tempfile::tempdir().unwrap();

        wipe_superseded_state(data_dir.path(), config_dir.path()).unwrap();
    }

    #[test]
    fn pairing_wipes_the_local_channel_renames() {
        // Display names keyed by the channels a redeem is about to replace
        // would otherwise outlive them and mislabel whatever reuses a name.
        let data_dir = Path::new("/data");

        let paths = paths_to_wipe_for_pairing(data_dir, Path::new("/config"));

        assert!(paths.contains(&data_dir.join("channel_renames")));
    }

    #[test]
    fn pairing_wipes_local_outboxes() {
        let data_dir = Path::new("/data");

        let paths = paths_to_wipe_for_pairing(data_dir, Path::new("/config"));

        assert!(paths.contains(&data_dir.join("outbox")));
    }

    #[test]
    fn clipboard_files_are_staged_on_the_channel_they_were_pasted_in() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.pdf");
        std::fs::write(&path, b"pdf bytes").unwrap();
        // Pasted in #general, but the user has since moved to #random.
        let mut app = AppState::new(
            [1; 32],
            vec!["general".to_string(), "random".to_string()],
            "random",
        );

        stage_files(&mut app, "general", &[path]);

        let staged = &app.channels[0].staged;
        assert_eq!(staged.len(), 1);
        assert_eq!(
            (staged[0].filename.as_str(), staged[0].size),
            ("report.pdf", 9)
        );
        assert!(app.channels[1].staged.is_empty());
    }

    #[test]
    fn a_refused_clipboard_file_is_reported_and_the_rest_are_still_staged() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.txt");
        std::fs::write(&good, b"ok").unwrap();
        let missing = dir.path().join("missing.txt");
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");

        stage_files(&mut app, "general", &[missing.clone(), good]);

        assert_eq!(app.active().staged.len(), 1);
        match app.active().messages.back() {
            Some(app::TranscriptLine::System(text)) => assert!(
                text.starts_with(&format!("can't send {}", missing.display())),
                "got: {text}"
            ),
            _ => panic!("expected a system notice about the missing file"),
        }
    }

    #[test]
    fn a_clipboard_image_is_staged_under_its_generated_name() {
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");

        stage_clipboard_image(
            &mut app,
            "general",
            "clipboard-1.png".to_string(),
            vec![1, 2, 3],
        );

        let staged = &app.active().staged;
        assert_eq!(staged.len(), 1);
        assert_eq!(
            (staged[0].filename.as_str(), staged[0].size),
            ("clipboard-1.png", 3)
        );
    }

    /// A file of `size` zero bytes called `name` in `dir`.
    fn file_of(dir: &Path, name: &str, size: usize) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, vec![0u8; size]).unwrap();
        path
    }

    /// The text of the newest system notice in `app`'s active channel.
    fn last_notice(app: &AppState) -> String {
        match app.active().messages.back() {
            Some(app::TranscriptLine::System(text)) => text.clone(),
            _ => panic!("expected a system notice"),
        }
    }

    #[test]
    fn a_file_over_the_size_limit_is_refused_when_staged_and_staged_once_the_limit_allows() {
        let dir = tempfile::tempdir().unwrap();
        let path = file_of(dir.path(), "big.bin", 2048);
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");
        app.load_size_limit(files::SizeLimit::Bytes(1024));

        stage_files(&mut app, "general", std::slice::from_ref(&path));

        assert!(app.active().staged.is_empty());
        let notice = last_notice(&app);
        assert!(
            notice.contains("too large") && notice.contains("/maxsize"),
            "got: {notice}"
        );

        app.load_size_limit(files::SizeLimit::Off);
        stage_files(&mut app, "general", &[path]);
        assert_eq!(app.active().staged.len(), 1);
    }

    #[test]
    fn a_dropped_file_answers_to_the_size_limit_like_any_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = file_of(dir.path(), "big.bin", 2048);
        let pasted = path.to_str().unwrap().to_string();
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");
        app.load_size_limit(files::SizeLimit::Bytes(1024));

        handle_paste(&mut app, &pasted);

        assert!(app.active().staged.is_empty());
        assert_eq!(
            app.active().input,
            "",
            "it was still a drop, so nothing is typed"
        );
        assert!(last_notice(&app).contains("too large"));

        app.load_size_limit(files::SizeLimit::Bytes(4096));
        handle_paste(&mut app, &pasted);
        assert_eq!(app.active().staged.len(), 1);
    }

    #[test]
    fn a_clipboard_image_answers_to_the_size_limit_too() {
        let name = || "clipboard-1.png".to_string();
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");
        app.load_size_limit(files::SizeLimit::Bytes(2));

        stage_clipboard_image(&mut app, "general", name(), vec![1, 2, 3]);

        assert!(app.active().staged.is_empty());
        let notice = last_notice(&app);
        assert!(
            notice.starts_with("can't paste image: too large (3 B, max 2 B)"),
            "got: {notice}"
        );
        assert!(notice.contains("/maxsize"), "got: {notice}");

        app.load_size_limit(files::SizeLimit::Off);
        stage_clipboard_image(&mut app, "general", name(), vec![1, 2, 3]);
        assert_eq!(app.active().staged.len(), 1);
    }

    #[test]
    fn pairing_leaves_the_size_limit_alone() {
        // A preference of this device, like the files root: a redeem
        // replaces the channels and history, not how this device is set up.
        let data_dir = Path::new("/data");

        let paths = paths_to_wipe_for_pairing(data_dir, Path::new("/config"));

        assert!(!paths.contains(&data_dir.join("size_limit")));
    }

    /// Writes a small file called `name` into `dir`; returns its path and
    /// the path escaped the way Terminal.app and iTerm2 paste a dropped
    /// file (a backslash before each space).
    fn dropped(dir: &Path, name: &str) -> (PathBuf, String) {
        let path = dir.join(name);
        std::fs::write(&path, b"contents").unwrap();
        let escaped = path.to_str().unwrap().replace(' ', "\\ ");
        (path, escaped)
    }

    #[test]
    fn a_pasted_file_path_is_staged_on_the_active_channel_not_typed_into_the_box() {
        let dir = tempfile::tempdir().unwrap();
        let (_, pasted) = dropped(dir.path(), "My Report.pdf");
        let mut app = AppState::new(
            [1; 32],
            vec!["general".to_string(), "random".to_string()],
            "random",
        );

        handle_paste(&mut app, &format!("{pasted} "));

        assert_eq!(app.channels[1].staged.len(), 1);
        assert_eq!(app.channels[1].staged[0].filename, "My Report.pdf");
        assert!(app.channels[0].staged.is_empty());
        assert_eq!(app.channels[1].input, "", "nothing is typed into the box");
    }

    #[test]
    fn ordinary_pasted_text_goes_into_the_box_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let (real, _) = dropped(dir.path(), "real.txt");
        let texts = [
            // A sentence that merely mentions a real file.
            format!("see {} for the details", real.display()),
            // A path that is not there.
            dir.path().join("missing.txt").display().to_string(),
            // A directory.
            dir.path().display().to_string(),
            "line one\nline two".to_string(),
        ];

        for text in texts {
            let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");

            handle_paste(&mut app, &text);

            assert_eq!(app.active().input, text);
            assert!(app.active().staged.is_empty(), "text: {text:?}");
        }
    }

    #[test]
    fn several_dropped_files_are_staged_together() {
        let dir = tempfile::tempdir().unwrap();
        let (_, first) = dropped(dir.path(), "a.txt");
        let (_, second) = dropped(dir.path(), "b b.txt");
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");

        handle_paste(&mut app, &format!("{first} {second} "));

        let names: Vec<&str> = app
            .active()
            .staged
            .iter()
            .map(|item| item.filename.as_str())
            .collect();
        assert_eq!(names, ["a.txt", "b b.txt"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_dropped_file_that_cannot_be_read_is_reported_and_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (_, readable) = dropped(dir.path(), "ok.txt");
        let (locked_path, locked) = dropped(dir.path(), "locked.txt");
        std::fs::set_permissions(&locked_path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&locked_path).is_ok() {
            // Running as root: permissions don't apply, so nothing is refused.
            return;
        }
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");

        handle_paste(&mut app, &format!("{locked} {readable}"));

        assert_eq!(app.active().staged.len(), 1);
        assert_eq!(app.active().staged[0].filename, "ok.txt");
        match app.active().messages.back() {
            Some(app::TranscriptLine::System(text)) => assert!(
                text.starts_with(&format!("can't send {}", locked_path.display())),
                "got: {text}"
            ),
            _ => panic!("expected a system notice about the unreadable file"),
        }
        assert_eq!(
            app.active().input,
            "",
            "it was still a drop, so nothing is typed"
        );
    }

    #[test]
    fn a_drop_while_the_switcher_is_open_is_a_plain_paste() {
        let dir = tempfile::tempdir().unwrap();
        let (path, pasted) = dropped(dir.path(), "report.pdf");
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");
        app.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL));

        handle_paste(&mut app, &pasted);

        assert!(
            app.active().staged.is_empty(),
            "files must not be staged behind an open overlay"
        );
        let switcher = app.switching.as_ref().expect("the switcher is still open");
        assert_eq!(switcher.query, path.to_str().unwrap());
    }

    #[test]
    fn a_drop_while_the_files_overlay_is_open_is_a_plain_paste() {
        let dir = tempfile::tempdir().unwrap();
        let (path, pasted) = dropped(dir.path(), "report.pdf");
        let mut app = AppState::new([1; 32], vec!["general".to_string()], "general");
        for c in "/files".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));

        handle_paste(&mut app, &pasted);

        assert!(
            app.active().staged.is_empty(),
            "files must not be staged behind an open overlay"
        );
        let overlay = app
            .viewing_files
            .as_ref()
            .expect("the files overlay is still open");
        assert_eq!(overlay.query, path.to_str().unwrap());
    }

    #[test]
    fn pairing_leaves_the_record_of_saved_files_alone() {
        // Where files were saved is a fact about this device's disk, which a
        // redeem doesn't touch -- like the files root, and unlike the
        // channels, history and names it replaces.
        let data_dir = Path::new("/data");

        let paths = paths_to_wipe_for_pairing(data_dir, Path::new("/config"));

        assert!(!paths.contains(&data_dir.join("saved_files")));
    }
}
