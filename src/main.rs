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

use app::AppState;
use net::{Net, NetEvent};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging()?;

    let bootstrap = parse_connect_args()?;
    let (net_tx, net_rx) = mpsc::channel(64);
    let net = Net::start(bootstrap, net_tx)
        .await
        .context("failed to start networking")?;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, net, net_rx).await;
    ratatui::restore();
    result
}

/// Parses repeated `--connect <endpoint-id>` flags into a list of ids to
/// dial on startup. This is a minimal stand-in for the invite ticket system
/// (build-order step 3): copy the id shown in a running instance's header
/// into a second instance's `--connect` flag to have them join the same
/// default channel.
fn parse_connect_args() -> anyhow::Result<Vec<String>> {
    let mut args = std::env::args().skip(1);
    let mut bootstrap = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--connect" => {
                let id = args.next().context("--connect requires an endpoint id")?;
                bootstrap.push(id);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(bootstrap)
}

/// Logs go to a file under the OS data dir instead of stdout, since stdout
/// is the alternate-screen TUI once the terminal is initialized.
fn init_logging() -> anyhow::Result<()> {
    let dirs = directories::ProjectDirs::from("dev", "leyline", "leyline")
        .ok_or_else(|| anyhow::anyhow!("could not determine a data directory for logs"))?;
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
