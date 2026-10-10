mod app;
mod backfill;
mod channel_registry;
mod contacts;
mod dm_registry;
mod files;
mod hyperlink;
mod identity;
mod markdown;
mod message;
mod net;
mod pairing;
mod search;
mod settings;
mod storage;
mod thread;
mod ticket;
mod ui;
mod version;

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use arboard::Clipboard;
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste, Event, EventStream, KeyEventKind,
};
use crossterm::execute;
use futures::StreamExt;
use iroh::EndpointAddr;
use iroh_tickets::Ticket;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{debug, info, warn};

use app::{AppState, InputAction};
use backfill::BackfillStore;
use channel_registry::ChannelRegistry;
use contacts::Contacts;
use dm_registry::DmRegistry;
use files::{downloads_dir, resolve_destination};
use net::{Net, NetEvent};
use search::{SearchOutcome, run_on_disk_scan};
use settings::Settings;
use storage::MessageStore;
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
    let dm_registry = DmRegistry::load(dirs.data_dir().join("dms"))
        .context("failed to initialize DM registry")?;
    let settings = Settings::load(dirs.data_dir().join("settings"))
        .context("failed to initialize settings")?;

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
        dm_registry,
        settings,
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
    result
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
    dm_registry: DmRegistry,
    settings: Settings,
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
/// loop".
async fn run(terminal: &mut ratatui::DefaultTerminal, session: Session) -> anyhow::Result<()> {
    let Session {
        net,
        mut net_rx,
        joined_channels,
        active_channel,
        store,
        backfill,
        mut registry,
        mut contacts,
        mut dm_registry,
        mut settings,
        data_dir,
    } = session;

    let mut app = AppState::new(net.our_id, joined_channels.clone(), &active_channel);
    app.load_contacts(contacts.all());
    app.load_settings(settings.bell_enabled());
    app.load_dm_registry(dm_registry.all());
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

    let mut term_events = EventStream::new();
    let mut tick = interval(Duration::from_millis(250));
    let mut history_heartbeat = interval(HISTORY_ANNOUNCE_INTERVAL);
    // Reports for `/search`'s background on-disk scans (see search.rs and
    // `InputAction::Search`) -- kept separate from `net_rx`/`NetEvent`
    // since this isn't network activity, following the same "async work
    // reports into the select loop" shape.
    let (search_tx, mut search_rx) = mpsc::channel::<SearchOutcome>(8);
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
                                if let Err(err) = backfill.record_message(&channel, &message).await {
                                    warn!(%channel, "failed to record sent message for backfill: {err}");
                                }
                                net.send(&channel, message);
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
                                app.push_system(format!("invite for #{channel}: {ticket}{suffix}"));
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
                            Some(InputAction::Nick(name)) => {
                                if let Err(err) = settings.set_nickname(name.clone()) {
                                    warn!("failed to persist nickname: {err}");
                                }
                                net.set_nickname(name);
                            }
                            Some(InputAction::Leave(channel)) => {
                                net.leave(&channel);
                                if let Err(err) = registry.forget_channel(&channel) {
                                    warn!(%channel, "failed to remove channel from registry: {err}");
                                }
                                if let Err(err) = dm_registry.forget_channel(&channel) {
                                    warn!(%channel, "failed to remove DM registry entry: {err}");
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
                            Some(InputAction::SaveFile { hash, filename, sender }) => {
                                let destination_dir = downloads_dir(&data_dir);
                                match resolve_destination(&destination_dir, &filename) {
                                    Ok(destination) => {
                                        net.save_file(hash, filename, sender, destination);
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
                            None => {}
                        }
                        dirty = true;
                    }
                    Event::Resize(_, _) => dirty = true,
                    Event::Paste(text) => {
                        app.paste_text(&text);
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
                    _ => None,
                };
                if let Some(name) = joined_name {
                    let secret = net
                        .secret_for(name)
                        .expect("just-joined channel must have a recorded secret");
                    if let Err(err) = registry.record_channel(name, secret) {
                        warn!(channel = %name, "failed to persist joined channel: {err}");
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
                        net.send(&channel, message.clone());
                        if let Err(err) = store.append(&channel, &message) {
                            warn!(%channel, "failed to persist sent file: {err}");
                        }
                        if let Err(err) = backfill.record_message(&channel, &message).await {
                            warn!(%channel, "failed to record sent file for backfill: {err}");
                        }
                        app.record_sent_message(&channel, message);
                    }
                    // `/paste` found real file(s) on the clipboard (e.g. a
                    // Finder/Explorer copy) -- see `net::Net::paste`.
                    // Shared exactly like `/send`, one message per path,
                    // so a multi-file clipboard copy lands as one message
                    // each.
                    NetEvent::ClipboardFiles(channel, paths) => {
                        for path in paths {
                            if let Some(message) = net.send_file(channel.clone(), path.display().to_string()) {
                                app.push_system(message);
                            }
                        }
                    }
                    // `/paste` found rendered image pixels (e.g. a
                    // screenshot) and already re-encoded them as PNG --
                    // see `net::Net::paste`/`net::encode_png`. Any
                    // synchronous error (e.g. too large) is reported
                    // immediately, mirroring `InputAction::SendFile`;
                    // success eventually arrives back as `FileReady`,
                    // handled above like any other locally-authored file.
                    NetEvent::ClipboardImage(channel, filename, bytes) => {
                        if let Some(message) = net.send_clipboard_image(channel, filename, bytes) {
                            app.push_system(message);
                        }
                    }
                    // `/paste` found no file or image, but did find plain
                    // text -- share it exactly like a typed-and-sent
                    // message (see `NetEvent::FileReady` above for why
                    // broadcast/persist/backfill/display always happen
                    // together for a locally-authored message).
                    NetEvent::ClipboardText(channel, text) => {
                        let message = app.compose_own_message(&text);
                        net.send(&channel, message.clone());
                        if let Err(err) = store.append(&channel, &message) {
                            warn!(%channel, "failed to persist pasted message: {err}");
                        }
                        if let Err(err) = backfill.record_message(&channel, &message).await {
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
                            if let Err(err) = backfill.record_message(&channel, &message).await {
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

    Ok(())
}
