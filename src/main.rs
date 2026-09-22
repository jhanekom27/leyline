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

use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use tokio::time::interval;

use app::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging()?;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal).await;
    ratatui::restore();
    result
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

    tracing_subscriber::fmt()
        .with_writer(Mutex::new(log_file))
        .with_ansi(false)
        .init();

    Ok(())
}

/// The "snappy async event loop": crossterm's async `EventStream` merged
/// with a periodic tick via `tokio::select!`, redrawing only when something
/// actually changed. `net::NetEvent` will join this select! once block 2
/// wires up iroh-gossip -- see concept.md's "snappy async event loop".
async fn run(terminal: &mut ratatui::DefaultTerminal) -> anyhow::Result<()> {
    let mut app = AppState::new();
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
                        app.handle_key(key);
                        dirty = true;
                    }
                    Event::Resize(_, _) => dirty = true,
                    _ => {}
                }
            }
            _ = tick.tick() => {
                // Reserved for cursor blink / relative-timestamp refresh.
            }
        }
    }

    Ok(())
}
