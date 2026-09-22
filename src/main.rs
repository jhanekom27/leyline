mod app;
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
use tracing::info;

use app::AppState;
use net::{Net, NetEvent};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dirs = project_dirs()?;
    init_logging(&dirs)?;

    let secret_key = identity::load_or_generate(&dirs.config_dir().join("identity"))
        .context("failed to load or generate identity")?;
    let join_ticket = parse_join_arg()?;

    let (net_tx, net_rx) = mpsc::channel(64);
    let net = Net::start(secret_key, join_ticket, net_tx)
        .await
        .context("failed to start networking")?;
    info!(ticket = %net.ticket, "ready");
    println!("your invite ticket (share it with others via --join <ticket>):");
    println!("{}", net.ticket);

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, net, net_rx).await;
    ratatui::restore();
    result
}

/// Parses an optional `--join <ticket>` flag used to bootstrap into the
/// default channel via another instance's invite ticket (build-order step
/// 3). Omit it to start alone, e.g. as the first peer others join.
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
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,leyline=debug,iroh_gossip=debug"));

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
) -> anyhow::Result<()> {
    let mut app = AppState::new(net.our_id);
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
                        if let Some(message) = app.handle_key(key) {
                            net.send(message);
                        }
                        dirty = true;
                    }
                    Event::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }
            Some(net_event) = net_rx.recv() => {
                app.handle_net_event(net_event);
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
