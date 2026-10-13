mod app;
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
mod markdown;
mod message;
mod net;
mod pairing;
mod picker;
mod search;
mod settings;
mod storage;
mod term_events;
mod thread;
mod ticket;
mod ui;
mod version;

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use arboard::Clipboard;
use crossterm::event::{DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind};
use crossterm::execute;
use iroh::EndpointAddr;
use iroh_tickets::Ticket;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{debug, info, warn};

use app::{AppState, InputAction};
use backfill::BackfillStore;
use channel_registry::ChannelRegistry;
use channel_renames::ChannelRenames;
use contacts::Contacts;
use direct_registry::DirectRegistry;
use dm_registry::DmRegistry;
use files::{downloads_dir, dropped_files, resolve_destination, stage_file, stage_image};
use forgotten_devices::ForgottenDevices;
use message::ChatMessage;
use net::{DEVICE_SYNC_CHANNEL, Net, NetEvent};
use picker::{FolderListing, read_folder, remember_folder, start_folder};
use search::{SearchOutcome, run_on_disk_scan};
use settings::Settings;
use storage::MessageStore;
use term_events::TermEvents;
use ticket::RoomSecret;

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
    let channel_renames = ChannelRenames::load(dirs.data_dir().join("channel_renames"))
        .context("failed to initialize channel renames")?;
    let dm_registry = DmRegistry::load(dirs.data_dir().join("dms"))
        .context("failed to initialize DM registry")?;
    let direct_registry = DirectRegistry::load(dirs.data_dir().join("direct_peers"))
        .context("failed to initialize direct-peer registry")?;
    let settings = Settings::load(dirs.data_dir().join("settings"))
        .context("failed to initialize settings")?;
    let forgotten_devices = ForgottenDevices::load(dirs.data_dir().join("forgotten_devices"))
        .context("failed to initialize forgotten devices")?;

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
    let known_channels = channel_snapshot(&registry);

    let (net_tx, net_rx) = mpsc::channel(64);
    let (net, joined_channels, active_channel) = Net::start(
        secret_key,
        user_key,
        args.join,
        known_channels,
        settings.nickname(),
        net_tx,
        backfill.clone(),
    )
    .await
    .context("failed to start networking")?;
    println!("invite tickets (paste into another instance's message box with /join <ticket>):");
    for name in &joined_channels {
        let ticket = net.ticket_for(name, dm_registry.is_dm_channel(name));
        info!(channel = %name, %ticket, "ready");
        println!("  #{name}: {ticket}");
    }

    let session = Session {
        net,
        net_rx,
        joined_channels,
        active_channel,
        store,
        backfill,
        registry,
        contacts,
        channel_renames,
        dm_registry,
        direct_registry,
        settings,
        forgotten_devices,
        data_dir,
    };

    let mut terminal = ratatui::init();
    // Bracketed paste lets a terminal paste arrive as one `Event::Paste`
    // instead of a burst of individual key events -- see
    // `AppState::paste_text` for why that matters. Best-effort: if the
    // terminal doesn't understand the enabling sequence, pastes just fall
    // back to today's character-by-character key events.
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let result = run(&mut terminal, session).await;
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
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

/// Snapshots every currently-known channel's name, room secret, and known
/// peer addresses -- the shape both `Net::start`'s `known_channels` and a
/// pairing ticket's payload (`net::Net::create_pairing_ticket`) need.
/// Shared so the two can never drift apart.
fn channel_snapshot(registry: &ChannelRegistry) -> Vec<(String, RoomSecret, Vec<EndpointAddr>)> {
    registry
        .channel_names()
        .into_iter()
        .map(|name| {
            let secret = registry
                .secret_for(&name)
                .expect("a recorded channel always has a room secret");
            let addrs = registry.bootstrap_for(&name);
            (name, secret, addrs)
        })
        .collect()
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
        data_dir.join("contacts"),
        data_dir.join("channel_renames"),
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
    joined_channels: Vec<String>,
    active_channel: String,
    store: MessageStore,
    backfill: BackfillStore,
    registry: ChannelRegistry,
    contacts: Contacts,
    channel_renames: ChannelRenames,
    dm_registry: DmRegistry,
    direct_registry: DirectRegistry,
    settings: Settings,
    forgotten_devices: ForgottenDevices,
    /// leyline's own data directory (`dirs.data_dir()`), needed so a
    /// `/save` with no resolvable OS Downloads folder still has somewhere
    /// to fall back to -- see `files::downloads_dir`.
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

/// How often each joined channel with any recorded history re-announces its
/// current root to the whole channel (see `net::Net::announce`), on top of
/// the immediate re-announce whenever a channel gains a gossip neighbor.
/// Much coarser than `tick` below, since this goes out over the network --
/// but still frequent enough that a peer only ever bootstrapped through one
/// specific channel member still hears from every other member within one
/// interval, not only whoever it happens to be a direct gossip neighbor of
/// (see concept.md's "Persistence & history backfill" section).
const HISTORY_ANNOUNCE_INTERVAL: Duration = Duration::from_secs(30);

/// Announces `channel`'s current history root to the whole channel, if
/// we've recorded any messages for it yet (see `BackfillStore::current_root`
/// and `net::Net::announce`) -- a no-op otherwise, since there's nothing to
/// offer. Shared by the per-neighbor fast path and the periodic heartbeat in
/// `run` below, so the two can't drift apart.
fn announce_history(net: &Net, backfill: &BackfillStore, channel: &str) {
    if let Some(root) = backfill.current_root(channel) {
        net.announce(channel, root);
    }
}

/// Sends a locally-authored `message` to `channel` via whichever transport
/// it actually belongs to: a point-to-point connection
/// (`net::Net::send_direct`) if `channel` is one (`app::AppState::
/// direct_peer_for`), else the ordinary gossip broadcast (`net::Net::
/// send`). Centralizing this one decision here, rather than duplicating it
/// at every call site that broadcasts a locally-authored message, is what
/// lets `/send`, `/paste`, and a plain typed message all work unmodified
/// over a point-to-point conversation -- none of them depend on gossip
/// specifically, only on reaching the right peer.
fn route_send(net: &Net, app: &AppState, channel: &str, message: ChatMessage) {
    match app.direct_peer_for(channel) {
        Some(peer) => net.send_direct(peer, message),
        None => net.send(channel, message),
    }
}

/// Stages real files on `channel` for review: the ones a `Ctrl+V`/`/paste`
/// found on the clipboard (`NetEvent::ClipboardFiles`, e.g. a
/// Finder/Explorer copy) -- on the channel the paste was for, which may no
/// longer be the active one by the time the clipboard read comes back --
/// and the ones a drag-and-drop pasted as paths (`handle_paste`). Each path
/// is checked exactly like a `/send`ed one (`files::stage_file`); one that
/// is refused (unreadable, too large) is reported as a system notice
/// without affecting the rest, so a multi-file copy or drop stages whatever
/// it can. Nothing is shared yet: that waits for `Enter`
/// (`app::AppState::send_staged`).
fn stage_files(app: &mut AppState, channel: &str, paths: &[PathBuf]) {
    for path in paths {
        match stage_file(path) {
            Ok(item) => app.stage_attachment(channel, item),
            Err(message) => app.push_system(message),
        }
    }
}

/// Stages a clipboard image (`NetEvent::ClipboardImage`, already
/// re-encoded as PNG) the same way `stage_files` stages files; the one
/// thing to check -- the size cap -- is reported immediately, like a
/// refused `/send`.
fn stage_clipboard_image(app: &mut AppState, channel: &str, filename: String, bytes: Vec<u8>) {
    match stage_image(filename, bytes) {
        Ok(item) => app.stage_attachment(channel, item),
        Err(message) => app.push_system(message),
    }
}

/// Handles a bracketed paste (`Event::Paste`). A terminal turns a
/// drag-and-drop into a paste of the dropped files' paths (or, in Warp's
/// case, types them out -- `term_events::TermEvents` turns that into the
/// same `Event::Paste`), so a paste that is nothing but existing files
/// (`files::dropped_files`) is a drop: the files are staged on the active
/// channel, like a `Ctrl+V` of a Finder copy, instead of landing in the
/// message box as a wall of path text.
/// Anything else is text for the message box, as it always was --
/// including a paste while the switcher, a pick or a pairing confirmation
/// has taken over input (`AppState::is_composing`), where staging files
/// would put them somewhere the user is not looking.
fn handle_paste(app: &mut AppState, text: &str) {
    if app.is_composing()
        && let Some(paths) = dropped_files(text)
    {
        let channel = app.active().name.clone();
        stage_files(app, &channel, &paths);
    } else {
        app.paste_text(text);
    }
}

/// Rings the terminal bell (ASCII BEL) to signal a newly-arrived message --
/// see `AppState::bell_enabled`, toggled via `/bell` and persisted through
/// settings.rs. A plain control byte, so it's safe to write straight to
/// stdout even with `ratatui::init()` owning the alternate screen: it
/// doesn't move the cursor or draw anything, and it's entirely up to the
/// terminal emulator how (or whether) to actually alert -- the same as a
/// shell's `printf '\a'`. Best-effort: a failed write here isn't worth
/// surfacing, unlike a storage/backfill error.
fn ring_bell() {
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(b"\x07").and_then(|()| stdout.flush());
}

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
        joined_channels,
        active_channel,
        store,
        backfill,
        mut registry,
        mut contacts,
        mut channel_renames,
        mut dm_registry,
        mut direct_registry,
        mut settings,
        mut forgotten_devices,
        data_dir,
    } = session;

    let mut app = AppState::new(net.our_id, joined_channels.clone(), &active_channel);
    app.load_self_user_id(net.our_user_id());
    app.load_contacts(contacts.all());
    app.load_renames(channel_renames.all());
    app.load_settings(settings.bell_enabled());
    app.load_dm_registry(dm_registry.all());
    app.load_forgotten_devices(forgotten_devices.all());
    app.load_browse_dir(start_folder(&data_dir));
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
        }
        match store.load(name) {
            Ok(history) => {
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
        let channel = net.open_direct(peer);
        if app.background_direct_channel(channel.clone(), peer) {
            match store.load(&channel) {
                Ok(history) => app.load_history(&channel, history),
                Err(err) => warn!(%channel, "failed to load message history: {err}"),
            }
        }
    }

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
    let mut dirty = true;

    while !app.should_quit {
        if dirty {
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
                                    if let Err(err) = direct_registry.record(peer) {
                                        warn!(%channel, "failed to persist direct peer registry entry: {err}");
                                    }
                                    match store.load(&channel) {
                                        Ok(history) => app.load_history(&channel, history),
                                        Err(err) => warn!(%channel, "failed to load message history: {err}"),
                                    }
                                }
                                if let Some(text) = text {
                                    let message = app.compose_own_message(&text);
                                    net.send_direct(peer, message.clone());
                                    if let Err(err) = store.append(&channel, &message) {
                                        warn!(%channel, "failed to persist sent message: {err}");
                                    }
                                    app.record_sent_message(&channel, message);
                                }
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
                                let snapshot = channel_snapshot(&registry);
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
                                if let Err(err) = store.delete(&channel) {
                                    warn!(%channel, "failed to delete channel history: {err}");
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
                                if let Some(message) = net.send_file(channel, path) {
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
                            Some(InputAction::SaveFile { hash, filename, providers }) => {
                                let destination_dir = downloads_dir(&data_dir);
                                match resolve_destination(&destination_dir, &filename) {
                                    Ok(destination) => {
                                        net.save_file(hash, filename, providers, destination);
                                    }
                                    Err(err) => {
                                        app.push_system(format!(
                                            "failed to save {filename}: {err}"
                                        ));
                                    }
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
                    Event::Resize(_, _) => dirty = true,
                    Event::Paste(text) => {
                        handle_paste(&mut app, &text);
                        dirty = true;
                    }
                    _ => {}
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
                    // A peer reached us over a point-to-point connection
                    // for the first time this session (see `direct.rs`) --
                    // create (or confirm) its background tab, mirroring
                    // `ChannelSyncJoined`'s "don't steal focus" semantics,
                    // and load its persisted history the first time, the
                    // same way the startup restore loop above does.
                    NetEvent::DirectJoined { channel, peer } => {
                        if app.background_direct_channel(channel.clone(), peer) {
                            if let Err(err) = direct_registry.record(peer) {
                                warn!(%channel, "failed to persist direct peer registry entry: {err}");
                            }
                            match store.load(&channel) {
                                Ok(history) => app.load_history(&channel, history),
                                Err(err) => warn!(%channel, "failed to load message history: {err}"),
                            }
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
                    // A `/save`d file failed to download or write to disk
                    // -- see `net::Net::save_file`. Logged for the same
                    // reason as `FileSendFailed` above.
                    NetEvent::FileSaveFailed { filename, error } => {
                        warn!(%filename, "failed to save file: {error}");
                        app.push_system(format!("failed to save {filename}: {error}"));
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
                            if app.direct_peer_for(&channel).is_none()
                                && let Err(err) = backfill.record_message(&channel, &message).await
                            {
                                warn!(%channel, "failed to record received message for backfill: {err}");
                            }
                            if app.bell_enabled {
                                ring_bell();
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
            _ = tick.tick() => {
                // Reserved for cursor blink / relative-timestamp refresh.
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
                }
            }
        }
    }

    net.shutdown().await?;

    Ok(app.restart_with_ticket)
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;

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
}
