mod app;
mod backfill;
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
use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{info, warn};

use app::{AppState, InputAction};
use backfill::BackfillStore;
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

    let secret_key = identity::load_or_generate(&dirs.config_dir().join("identity"))
        .context("failed to load or generate identity")?;
    let join_ticket = parse_join_arg()?;

    let (net_tx, net_rx) = mpsc::channel(64);
    let (net, joined_channels) = Net::start(secret_key, join_ticket, net_tx, backfill.clone())
        .await
        .context("failed to start networking")?;
    println!("invite tickets (share via --join <ticket> or the in-app /join <ticket> command):");
    for name in &joined_channels {
        let ticket = net.ticket_for(name);
        info!(channel = %name, %ticket, "ready");
        println!("  #{name}: {ticket}");
    }

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, net, net_rx, joined_channels, store, backfill).await;
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

/// The "snappy async event loop": crossterm's async `EventStream` and the
/// network's `NetEvent`s merged via `tokio::select!`, redrawing only when
/// something actually changed -- see concept.md's "snappy async event
/// loop".
async fn run(
    terminal: &mut ratatui::DefaultTerminal,
    net: Net,
    mut net_rx: mpsc::Receiver<NetEvent>,
    joined_channels: Vec<String>,
    store: MessageStore,
    backfill: BackfillStore,
) -> anyhow::Result<()> {
    let mut app = AppState::new(net.our_id, joined_channels.clone());
    for name in &joined_channels {
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
                                if let Some(error) = net.join(arg) {
                                    app.push_system(error);
                                }
                            }
                            Some(InputAction::Invite(channel)) => {
                                let ticket = net.ticket_for(&channel);
                                app.push_system(format!("invite for #{channel}: {ticket}"));
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
                // neighbor, so we can also announce our history root to it
                // -- see backfill.rs and concept.md's "Persistence and
                // offline history" section. `app.handle_net_event` below
                // still separately updates presence for this same event.
                if let NetEvent::PeerJoined(channel, _) = &net_event
                    && let Some(root) = backfill.current_root(channel)
                {
                    net.announce(channel, root);
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
