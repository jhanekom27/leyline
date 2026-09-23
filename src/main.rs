mod app;
mod backfill;
mod channel_registry;
mod contacts;
mod identity;
mod message;
mod net;
mod storage;
mod ticket;
mod ui;

use std::fs::OpenOptions;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::Context;
use arboard::Clipboard;
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{debug, info, warn};

use app::{AppState, InputAction};
use backfill::BackfillStore;
use channel_registry::ChannelRegistry;
use contacts::Contacts;
use net::{Net, NetEvent};
use storage::MessageStore;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dirs = project_dirs()?;
    init_logging(&dirs)?;

    let store = MessageStore::new(dirs.data_dir().join("messages"))
        .context("failed to initialize message storage")?;
    let backfill = BackfillStore::new(dirs.data_dir().join("blobs"))
        .await
        .context("failed to initialize history backfill storage")?;
    let registry = ChannelRegistry::load(dirs.data_dir().join("channels"))
        .context("failed to initialize channel registry")?;
    let contacts = Contacts::load(dirs.data_dir().join("contacts"))
        .context("failed to initialize contacts")?;

    let secret_key = identity::load_or_generate(&dirs.config_dir().join("identity"))
        .context("failed to load or generate identity")?;
    let join_ticket = parse_join_arg()?;

    // What to rejoin from the previous session -- see channel_registry.rs
    // and `Net::start`. Empty on a first run (just "general" then).
    let known_channels = registry
        .channel_names()
        .into_iter()
        .map(|name| {
            let secret = registry
                .secret_for(&name)
                .expect("a recorded channel always has a room secret");
            let addrs = registry.bootstrap_for(&name);
            (name, secret, addrs)
        })
        .collect();

    let (net_tx, net_rx) = mpsc::channel(64);
    let (net, joined_channels, active_channel) = Net::start(
        secret_key,
        join_ticket,
        known_channels,
        net_tx,
        backfill.clone(),
    )
    .await
    .context("failed to start networking")?;
    println!("invite tickets (paste into another instance's message box with /join <ticket>):");
    for name in &joined_channels {
        let ticket = net.ticket_for(name);
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
    };

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, session).await;
    ratatui::restore();
    result
}

/// Parses an optional `--join <ticket>` flag used to bootstrap into another
/// instance's channel via its invite ticket. Omit it to start alone with
/// just the default "general" channel, e.g. as the first peer others join.
fn parse_join_arg() -> anyhow::Result<Option<String>> {
    let mut args = std::env::args().skip(1);
    let mut ticket = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--join" => {
                anyhow::ensure!(ticket.is_none(), "--join can only be specified once");
                ticket = Some(args.next().context("--join requires a ticket")?);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(ticket)
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
    } = session;

    let mut app = AppState::new(net.our_id, joined_channels.clone(), &active_channel);
    app.load_contacts(contacts.all());
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

    let mut term_events = EventStream::new();
    let mut tick = interval(Duration::from_millis(250));
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
                            Some(InputAction::Invite(channel)) => {
                                let ticket = net.ticket_for(&channel);
                                let copied = clipboard.as_mut().is_some_and(|clipboard| {
                                    clipboard.set_text(ticket.clone()).is_ok()
                                });
                                let suffix = if copied { " (copied to clipboard)" } else { "" };
                                app.push_system(format!("invite for #{channel}: {ticket}{suffix}"));
                            }
                            Some(InputAction::Alias(id, name)) => {
                                if let Err(err) = contacts.set(id, name) {
                                    warn!("failed to persist pet name: {err}");
                                }
                            }
                            Some(InputAction::Nick(name)) => {
                                net.set_nickname(name);
                            }
                            None => {}
                        }
                        dirty = true;
                    }
                    Event::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }
            Some(net_event) = net_rx.recv() => {
                // Peek (without consuming) for a channel gaining a gossip
                // neighbor, so we can also announce our history root and
                // our current nickname (if any) to it -- see backfill.rs,
                // concept.md's "Persistence & history backfill" section,
                // and `Net::announce_nickname`. `app.handle_net_event`
                // below still separately updates presence for this same
                // event.
                if let NetEvent::PeerJoined(channel, _) = &net_event {
                    if let Some(root) = backfill.current_root(channel) {
                        net.announce(channel, root);
                    }
                    net.announce_nickname(channel);
                }

                // Also peek for a channel finishing a runtime `/join`, so
                // it's persisted to the channel registry -- see
                // channel_registry.rs -- and rejoined automatically on a
                // future restart. Channels joined at startup are already
                // in the registry (see the loop above), so this only
                // matters for new ones joined during this session.
                if let NetEvent::Joined(name) = &net_event {
                    let secret = net
                        .secret_for(name)
                        .expect("just-joined channel must have a recorded secret");
                    if let Err(err) = registry.record_channel(name, secret) {
                        warn!(channel = %name, "failed to persist joined channel: {err}");
                    }
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
                    other => {
                        if let Some((channel, message)) = app.handle_net_event(other) {
                            if let Err(err) = store.append(&channel, &message) {
                                warn!(%channel, "failed to persist received message: {err}");
                            }
                            if let Err(err) = backfill.record_message(&channel, &message).await {
                                warn!(%channel, "failed to record received message for backfill: {err}");
                            }
                        }
                    }
                }
                dirty = true;
            }
            _ = tick.tick() => {
                // Reserved for cursor blink / relative-timestamp refresh.
            }
        }
    }

    net.shutdown().await?;

    Ok(())
}
