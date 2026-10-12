//! The terminal's input events, with one fix: a drag-and-drop that a
//! terminal types out instead of pasting.
//!
//! A terminal has no drop protocol. Most paste the dropped files' paths --
//! bracketed, if the program asked for that, so it arrives as one
//! `Event::Paste` (see `main::handle_paste`). Warp does not: its drop
//! handler writes the shell-escaped paths straight to the pty as if they
//! were typed (`drag_and_drop_files` ends in `typed_characters_on_terminal`,
//! where `Cmd+V` goes through `paste`, which brackets), so all leyline sees
//! is a key press per character and no paste at all.
//!
//! Typing and a drop differ on the wire, though. A person delivers one key
//! at a time; a drop delivers the whole path in a single write, so every key
//! press of it is already waiting when the first is read. [`TermEvents`]
//! gathers what arrived together, and a run of plain characters that spells
//! out nothing but existing files (`files::dropped_files`) comes out as the
//! `Event::Paste` it would have been. Every other event comes out untouched,
//! in order. The filesystem decides, so ordinary typing is never mistaken
//! for a drop: it would have to arrive in one piece *and* be the absolute
//! path of a file that exists.
//!
//! This leans on a drop arriving in one piece. Were a terminal ever to
//! trickle one out, each piece would come out as typed text -- what
//! happened to every typed-out drop before this existed.

use std::collections::VecDeque;
use std::io;
use std::task::Poll;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures::StreamExt;
use tracing::debug;

use crate::files::dropped_files;

/// crossterm's `EventStream`, with a drop that was typed out turned into the
/// paste it stands for -- see the module docs.
pub struct TermEvents {
    stream: EventStream,
    /// Events read off the stream but not handed out yet, oldest first.
    /// Refilled only once empty, so each batch is judged as a whole, once.
    ready: VecDeque<Event>,
}

impl TermEvents {
    pub fn new() -> Self {
        Self {
            stream: EventStream::new(),
            ready: VecDeque::new(),
        }
    }

    /// The next event, waiting for one if none is ready. Safe to drop while
    /// waiting (as `tokio::select!` does): the only thing awaited is the
    /// stream's own `next`, and everything read after it is read without
    /// yielding. An error is returned the moment it is met, ahead of any
    /// events read alongside it -- the caller gives up on the first one, so
    /// they would never be handled anyway.
    pub async fn next(&mut self) -> Option<io::Result<Event>> {
        if self.ready.is_empty() {
            let first = match self.stream.next().await? {
                Ok(event) => event,
                Err(err) => return Some(Err(err)),
            };
            let mut batch = vec![first];
            // Whatever else is already waiting. `poll!`, not `now_or_never`: a
            // stream that answers `Pending` keeps the waker it was polled with
            // to wake the task later, and a no-op one would leave the next key
            // press unseen until the loop's next tick.
            while let Poll::Ready(Some(next)) = futures::poll!(self.stream.next()) {
                match next {
                    Ok(event) => batch.push(event),
                    Err(err) => return Some(Err(err)),
                }
            }
            self.ready = typed_drops_as_pastes(batch);
        }
        self.ready.pop_front().map(Ok)
    }
}

/// `events` with each run of typed characters that spells out dropped files
/// replaced by the single `Event::Paste` a bracketing terminal would have
/// sent for it; everything else as it was, in order.
fn typed_drops_as_pastes(events: Vec<Event>) -> VecDeque<Event> {
    let mut out = VecDeque::with_capacity(events.len());
    let mut run = Vec::new();
    for event in events {
        if typed_char(&event).is_some() {
            run.push(event);
        } else {
            end_run(&mut run, &mut out);
            out.push_back(event);
        }
    }
    end_run(&mut run, &mut out);
    out
}

/// Moves a finished `run` of typed characters (possibly none) to `out`: as
/// one paste if it spells out dropped files, else as the key presses it is.
fn end_run(run: &mut Vec<Event>, out: &mut VecDeque<Event>) {
    let text: String = run.iter().filter_map(typed_char).collect();
    if dropped_files(&text).is_some() {
        // The size only: file names don't belong in a log.
        debug!(bytes = text.len(), "typed-out drop read as a paste");
        run.clear();
        out.push_back(Event::Paste(text));
    } else {
        out.extend(run.drain(..));
    }
}

/// The character a key press types: a `Char` with nothing held down but
/// Shift, which is how a capital letter is reported.
fn typed_char(event: &Event) -> Option<char> {
    match event {
        Event::Key(KeyEvent {
            code: KeyCode::Char(c),
            modifiers,
            kind: KeyEventKind::Press,
            ..
        }) if modifiers.difference(KeyModifiers::SHIFT).is_empty() => Some(*c),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    /// The key press a terminal reports for typing `c`.
    fn key(c: char) -> Event {
        let modifiers = if c.is_uppercase() {
            KeyModifiers::SHIFT
        } else {
            KeyModifiers::NONE
        };
        Event::Key(KeyEvent::new(KeyCode::Char(c), modifiers))
    }

    /// The key presses that arrive when `text` is typed out.
    fn typed(text: &str) -> Vec<Event> {
        text.chars().map(key).collect()
    }

    /// `events` as `TermEvents` hands them out.
    fn coalesced(events: Vec<Event>) -> Vec<Event> {
        typed_drops_as_pastes(events).into()
    }

    /// Writes a one-byte file called `name` into `dir` and returns its path.
    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"x").unwrap();
        path
    }

    /// `path` as Warp types a dropped file: a backslash before every
    /// character a POSIX shell treats specially.
    fn warp_escaped(path: &Path) -> String {
        path.to_str()
            .unwrap()
            .chars()
            .flat_map(|c| {
                let special = " \"$'\\#=[]!><|;{}()*?&`~".contains(c);
                special.then_some('\\').into_iter().chain([c])
            })
            .collect()
    }

    #[test]
    fn a_typed_out_path_is_read_as_the_paste_it_stands_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = touch(dir.path(), "My Report (final) #1.pdf");
        // What Warp writes: the escaped path and a space, capitals and all.
        let text = format!("{} ", warp_escaped(&path));

        assert_eq!(coalesced(typed(&text)), [Event::Paste(text)]);
    }

    #[test]
    fn several_typed_out_paths_are_one_paste() {
        let dir = tempfile::tempdir().unwrap();
        let first = touch(dir.path(), "a.txt");
        let second = touch(dir.path(), "b b.txt");
        let text = format!("{} {} ", warp_escaped(&first), warp_escaped(&second));

        assert_eq!(coalesced(typed(&text)), [Event::Paste(text)]);
    }

    #[test]
    fn typed_text_that_is_not_a_drop_stays_key_presses() {
        let dir = tempfile::tempdir().unwrap();
        let real = touch(dir.path(), "real.txt");
        let texts = [
            "hello world".to_string(),
            // A sentence that merely mentions a real file.
            format!("see {} for the details", warp_escaped(&real)),
            // A path that is not there.
            warp_escaped(&dir.path().join("missing.txt")),
            // A directory.
            warp_escaped(dir.path()),
            // A lone slash, the start of a command or a path.
            "/".to_string(),
        ];

        for text in texts {
            assert_eq!(coalesced(typed(&text)), typed(&text), "text: {text:?}");
        }
    }

    #[test]
    fn a_drop_keeps_its_place_among_other_events() {
        let dir = tempfile::tempdir().unwrap();
        let path = touch(dir.path(), "a.txt");
        let text = format!("{} ", warp_escaped(&path));
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let mut events = vec![Event::Resize(80, 24)];
        events.extend(typed("ab"));
        events.push(enter.clone());
        events.extend(typed(&text));
        events.push(enter.clone());

        let mut expected = vec![Event::Resize(80, 24)];
        expected.extend(typed("ab"));
        expected.extend([enter.clone(), Event::Paste(text), enter]);
        assert_eq!(coalesced(events), expected);
    }

    #[test]
    fn a_path_run_into_other_typing_is_not_a_drop() {
        // All or nothing, as for a bracketed paste: text typed in the same
        // breath makes the whole run text.
        let dir = tempfile::tempdir().unwrap();
        let path = touch(dir.path(), "a.txt");
        let text = format!("look {} ", warp_escaped(&path));

        assert_eq!(coalesced(typed(&text)), typed(&text));
    }

    #[test]
    fn a_key_press_with_a_modifier_or_that_is_not_a_press_is_not_typing() {
        let dir = tempfile::tempdir().unwrap();
        let path = touch(dir.path(), "a.txt");
        let text = format!("{} ", warp_escaped(&path));
        let mut release = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
        release.kind = KeyEventKind::Release;
        let interlopers = [
            Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)),
            Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
            Event::Key(release),
        ];

        for interloper in interlopers {
            let mut events = typed(&text);
            events.insert(events.len() / 2, interloper);
            assert_eq!(coalesced(events.clone()), events);
        }
    }

    #[test]
    fn no_events_come_out_as_none() {
        assert_eq!(coalesced(Vec::new()), []);
    }
}
