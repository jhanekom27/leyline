//! The in-TUI file picker behind a bare `/send` -- see features.md's "File
//! picker" entry. A keyboard-driven browser in the style of the `Ctrl+P`
//! jump-to overlay: it needs no GUI dialog, so it works over SSH and on
//! Linux, and it takes away the friction of typing an exact path.
//!
//! [`FilePicker`] is pure state, like `thread::ThreadView` and
//! `search::SearchResults`: app.rs owns one while the picker is open and
//! feeds it keys, ui.rs draws it, and neither touches the filesystem.
//! Reading a folder ([`read_folder`]) is the one piece of I/O, and main.rs
//! runs it on tokio's blocking pool -- the same shape as `/search`'s
//! on-disk scan -- so a huge or slow folder never blocks input: the picker
//! shows what it has, answers keys, and applies the listing whenever it
//! arrives.

use std::cmp::Ordering;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// File in leyline's data directory that remembers the folder files were
/// last chosen from. A one-line text file of its own, deliberately not a
/// field of the postcard `settings` file: postcard isn't self-describing,
/// so a longer struct would fail to decode an existing file and silently
/// reset the nickname and bell setting saved in it.
const REMEMBERED_FOLDER: &str = "browse_dir";

/// One row of a folder listing: a folder, or a regular file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: Kind,
    /// `name` lowercased once, when the folder is read, so narrowing the
    /// list on every typed character doesn't lowercase every name again.
    lower: String,
}

impl Entry {
    pub fn new(name: String, kind: Kind) -> Self {
        let lower = name.to_lowercase();
        Self { name, kind, lower }
    }

    pub fn is_folder(&self) -> bool {
        self.kind == Kind::Folder
    }
}

/// What an `Entry` is. Only a file has a size and a modified time, and
/// only files can be chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File { size: u64, modified: SystemTime },
}

/// What a background `read_folder` reports back to main.rs's select loop,
/// and from there to `FilePicker::apply`.
#[derive(Debug)]
pub struct FolderListing {
    /// Which read this answers -- see `FilePicker::read`.
    pub request: u64,
    pub result: Result<Vec<Entry>, String>,
}

/// What a key press asks the caller (`app::AppState`) to do next.
#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    /// Nothing beyond what the picker already did to itself.
    Stay,
    /// Close the picker without choosing anything.
    Cancel,
    /// Read this folder (`FilePicker::read`) and show it.
    Open(PathBuf),
    /// Close the picker and stage these files.
    Choose(Vec<PathBuf>),
}

/// The open file picker -- see `app::AppState::picking_file`.
pub struct FilePicker {
    /// The channel the picker was opened on, which is where chosen files
    /// are staged: not necessarily the active one by the time `Enter` is
    /// pressed, since a join can switch it meanwhile.
    pub channel: String,
    /// The folder shown.
    dir: PathBuf,
    /// Everything in `dir`, in display order (see `read_folder`).
    entries: Vec<Entry>,
    /// What's been typed to narrow the folder by name.
    query: String,
    /// Indices into `entries` of those whose name contains `query`: the
    /// rows on screen, in the same order.
    shown: Vec<usize>,
    /// Position in `shown` of the highlighted row. Always inside it (0
    /// while it's empty): only `refilter` and `move_highlight` change it
    /// or `shown`.
    selected: usize,
    /// Files marked with `Space`, in the order they were marked, as full
    /// paths -- so they are kept when the folder changes.
    marked: Vec<PathBuf>,
    /// The read in flight, if any.
    reading: Option<Read>,
    /// A problem worth telling the user about -- an unreadable folder, a
    /// `Space` on a folder -- until their next key press.
    notice: Option<String>,
}

/// A folder being read -- see `FilePicker::read`.
#[derive(Debug)]
struct Read {
    request: u64,
    dir: PathBuf,
    /// The entry to highlight once the folder arrives: the folder just
    /// left, when going up.
    select: Option<String>,
}

impl FilePicker {
    /// A picker for `channel` showing `dir`, whose first read is `request`.
    /// It shows nothing until `apply` is given that read's answer.
    pub fn new(channel: String, dir: PathBuf, request: u64) -> Self {
        let mut picker = Self {
            channel,
            dir: dir.clone(),
            entries: Vec::new(),
            query: String::new(),
            shown: Vec::new(),
            selected: 0,
            marked: Vec::new(),
            reading: None,
            notice: None,
        };
        picker.read(request, dir);
        picker
    }

    /// The folder shown.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// What's been typed to narrow the folder.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The rows on screen, top to bottom.
    pub fn visible(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.shown.iter().map(|&index| &self.entries[index])
    }

    /// Position among `visible` of the highlighted row.
    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Whether the folder shown holds nothing the picker lists, typed
    /// filter aside.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether a folder is still being read.
    pub fn is_reading(&self) -> bool {
        self.reading.is_some()
    }

    /// The notice to show instead of the key hints, if any.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    /// How many files are marked, across every folder visited.
    pub fn marked_count(&self) -> usize {
        self.marked.len()
    }

    /// Whether `entry`, of the folder shown, is marked.
    pub fn is_marked(&self, entry: &Entry) -> bool {
        self.marked.contains(&self.dir.join(&entry.name))
    }

    /// Notes that `dir` is being read, as `request`: `apply` shows it once
    /// answered, and ignores an answer to any earlier read, so a slow
    /// folder can't land on top of one asked for since. When `dir` is an
    /// ancestor of the folder on screen (going up), the highlight will land
    /// on the child just left.
    pub fn read(&mut self, request: u64, dir: PathBuf) {
        let select = self
            .dir
            .strip_prefix(&dir)
            .ok()
            .and_then(|below| below.components().next())
            .map(|child| child.as_os_str().to_string_lossy().into_owned());
        self.reading = Some(Read {
            request,
            dir,
            select,
        });
    }

    /// Applies a finished read -- but only if it is the one awaited. A
    /// folder arriving replaces the listing and highlights its first row (or
    /// the folder just left). A different folder also starts with a clean
    /// filter, but the folder the picker opened on keeps what was typed
    /// while it was being read: a big one can take a while, and the keys
    /// typed meanwhile are meant for it. A folder that couldn't be read
    /// leaves the old listing as it was and says why, so the picker stays
    /// usable.
    pub fn apply(&mut self, listing: FolderListing) {
        let Some(read) = self.reading.take_if(|read| read.request == listing.request) else {
            return;
        };
        match listing.result {
            Ok(entries) => {
                if read.dir != self.dir {
                    self.query.clear();
                }
                self.dir = read.dir;
                self.entries = entries;
                self.notice = None;
                self.refilter();
                let left = read.select.and_then(|name| {
                    self.shown
                        .iter()
                        .position(|&index| self.entries[index].name == name)
                });
                if let Some(row) = left {
                    self.selected = row;
                }
            }
            Err(message) => self.notice = Some(message),
        }
    }

    /// Applies `edit` to the typed filter and narrows the folder to it,
    /// highlighting the first match. The one place the filter changes, so
    /// a typed character, a `Backspace` and a paste can't disagree about
    /// where the highlight lands.
    pub fn edit_query(&mut self, edit: impl FnOnce(&mut String)) {
        edit(&mut self.query);
        self.refilter();
    }

    /// Handles a key press. Up and Down move the highlight; `Enter` opens
    /// a folder or, on a file, chooses the marked files (the highlighted
    /// one if none are marked); `Space` marks or unmarks the highlighted
    /// file, so the filter can't hold a space (a pasted one can);
    /// `Backspace` deletes a typed character and, with none left, goes up a
    /// level; any other character narrows the folder; `Esc` cancels.
    pub fn handle_key(&mut self, key: KeyEvent) -> Step {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        self.notice = None;
        match key.code {
            KeyCode::Esc => Step::Cancel,
            KeyCode::Up => self.move_highlight(-1),
            KeyCode::Down => self.move_highlight(1),
            KeyCode::Enter => self.enter(),
            KeyCode::Backspace => self.backspace(),
            KeyCode::Char(' ') if !ctrl => self.toggle_mark(),
            KeyCode::Char(c) if !ctrl => {
                self.edit_query(|query| query.push(c));
                Step::Stay
            }
            _ => Step::Stay,
        }
    }

    /// The highlighted row, if any row is shown.
    fn highlighted(&self) -> Option<&Entry> {
        self.shown
            .get(self.selected)
            .map(|&index| &self.entries[index])
    }

    /// Re-derives `shown` from the filter and highlights its first row.
    fn refilter(&mut self) {
        let query = self.query.to_lowercase();
        self.shown = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.lower.contains(&query))
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
    }

    /// Moves the highlight by `delta` rows, stopping at both ends rather
    /// than wrapping, like the switcher's.
    fn move_highlight(&mut self, delta: isize) -> Step {
        let last = self.shown.len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        Step::Stay
    }

    fn enter(&self) -> Step {
        match self.highlighted() {
            Some(entry) if entry.is_folder() => Step::Open(self.dir.join(&entry.name)),
            Some(entry) if self.marked.is_empty() => Step::Choose(vec![self.dir.join(&entry.name)]),
            // With nothing shown the marks can still be chosen -- a filter
            // that matches nothing shouldn't strand them.
            Some(_) | None if !self.marked.is_empty() => Step::Choose(self.marked.clone()),
            _ => Step::Stay,
        }
    }

    fn backspace(&mut self) -> Step {
        if self.query.is_empty() {
            return match self.dir.parent() {
                Some(parent) => Step::Open(parent.to_path_buf()),
                None => Step::Stay,
            };
        }
        self.edit_query(|query| {
            query.pop();
        });
        Step::Stay
    }

    fn toggle_mark(&mut self) -> Step {
        let path = match self.highlighted() {
            Some(entry) if !entry.is_folder() => self.dir.join(&entry.name),
            Some(_) => {
                self.notice = Some("only files can be marked -- Enter opens a folder".to_string());
                return Step::Stay;
            }
            None => return Step::Stay,
        };
        match self.marked.iter().position(|marked| *marked == path) {
            Some(at) => {
                self.marked.remove(at);
            }
            None => self.marked.push(path),
        }
        Step::Stay
    }
}

/// Lists `dir` for the picker: folders first (by name, ignoring case),
/// then files newest first (by modified time, then name), so recent
/// downloads and screenshots are on top. Left out: dot-files and
/// dot-folders, and anything that is neither a folder nor a regular file
/// (sockets, devices) or can't be looked at (a broken link, or one that
/// vanished meanwhile) -- none of those can be sent anyway. A link counts
/// as what it points to, so a link to a folder opens like one.
///
/// Synchronous, so the caller (main.rs) runs it inside a
/// `tokio::task::spawn_blocking`: a huge or slow folder -- a network mount
/// -- can't be assumed cheap enough for the async event loop. A folder that
/// can't be read is an error naming it and why -- by its own name, which
/// the user has just pressed `Enter` on and which is all the one-row notice
/// has room for besides the reason.
pub fn read_folder(dir: &Path) -> Result<Vec<Entry>, String> {
    let name = dir.file_name().map_or_else(
        || dir.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let cant_open = |err: io::Error| format!("can't open {name}: {err}");
    let mut entries = Vec::new();
    for item in std::fs::read_dir(dir).map_err(cant_open)? {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(item.path()) else {
            continue;
        };
        let kind = if metadata.is_dir() {
            Kind::Folder
        } else if metadata.is_file() {
            Kind::File {
                size: metadata.len(),
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            }
        } else {
            continue;
        };
        entries.push(Entry::new(name, kind));
    }
    entries.sort_by(display_order);
    Ok(entries)
}

/// The order a folder is listed in: see `read_folder`.
fn display_order(a: &Entry, b: &Entry) -> Ordering {
    let by_name = || a.lower.cmp(&b.lower).then_with(|| a.name.cmp(&b.name));
    match (a.kind, b.kind) {
        (Kind::Folder, Kind::Folder) => by_name(),
        (Kind::Folder, Kind::File { .. }) => Ordering::Less,
        (Kind::File { .. }, Kind::Folder) => Ordering::Greater,
        (Kind::File { modified: a, .. }, Kind::File { modified: b, .. }) => {
            b.cmp(&a).then_with(by_name)
        }
    }
}

/// Where the picker opens when leyline starts: the folder files were last
/// chosen from, if it's still there, else the OS Downloads folder, home,
/// the current folder, or -- for a system with none of those -- the root.
pub fn start_folder(data_dir: &Path) -> PathBuf {
    let dirs = directories::UserDirs::new();
    first_folder([
        remembered_folder(data_dir),
        dirs.as_ref()
            .and_then(|dirs| dirs.download_dir().map(Path::to_path_buf)),
        dirs.as_ref().map(|dirs| dirs.home_dir().to_path_buf()),
        std::env::current_dir().ok(),
    ])
}

/// The first of `candidates` that is a folder, else the root.
fn first_folder(candidates: impl IntoIterator<Item = Option<PathBuf>>) -> PathBuf {
    candidates
        .into_iter()
        .flatten()
        .find(|dir| dir.is_dir())
        .unwrap_or_else(|| PathBuf::from(std::path::MAIN_SEPARATOR_STR))
}

/// The folder `remember_folder` last recorded in `data_dir`, if any. Not
/// checked to still exist: `start_folder` does that, and falls back.
fn remembered_folder(data_dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(data_dir.join(REMEMBERED_FOLDER)).ok()?;
    Some(PathBuf::from(text.trim_end_matches('\n')))
}

/// Records `dir` as the folder the picker should open in next time -- see
/// `start_folder`. Called when files are chosen from it, not on every
/// folder visited or on a cancel.
pub fn remember_folder(data_dir: &Path, dir: &Path) -> io::Result<()> {
    let text = dir.to_str().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "the folder's name isn't valid UTF-8",
        )
    })?;
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(data_dir.join(REMEMBERED_FOLDER), text)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::time::Duration;

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn typed(picker: &mut FilePicker, text: &str) {
        for c in text.chars() {
            picker.handle_key(key(KeyCode::Char(c)));
        }
    }

    fn file(name: &str) -> Entry {
        Entry::new(
            name.to_string(),
            Kind::File {
                size: 10,
                modified: SystemTime::UNIX_EPOCH,
            },
        )
    }

    fn folder(name: &str) -> Entry {
        Entry::new(name.to_string(), Kind::Folder)
    }

    /// A picker on `dir` that has been given `entries` for it.
    fn picker_on(dir: &str, entries: Vec<Entry>) -> FilePicker {
        let mut picker = FilePicker::new("general".to_string(), PathBuf::from(dir), 1);
        picker.apply(FolderListing {
            request: 1,
            result: Ok(entries),
        });
        picker
    }

    /// A picker on `/pics` holding a folder and three files.
    fn pics() -> FilePicker {
        picker_on(
            "/pics",
            vec![folder("docs"), file("a.txt"), file("b.txt"), file("c.txt")],
        )
    }

    fn shown(picker: &FilePicker) -> Vec<&str> {
        picker.visible().map(|entry| entry.name.as_str()).collect()
    }

    fn highlighted(picker: &FilePicker) -> Option<&str> {
        picker.highlighted().map(|entry| entry.name.as_str())
    }

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|name| Path::new("/pics").join(name))
            .collect()
    }

    // -- read_folder ---------------------------------------------------------

    /// Writes an empty file called `name` into `dir`, last modified `age`
    /// ago.
    fn touch(dir: &Path, name: &str, age: Duration) {
        let file = File::create(dir.join(name)).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    fn listed(dir: &Path) -> Vec<String> {
        read_folder(dir)
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect()
    }

    #[test]
    fn folders_come_first_by_name_then_files_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("zebra")).unwrap();
        std::fs::create_dir(dir.path().join("Apple")).unwrap();
        touch(dir.path(), "old.txt", Duration::from_secs(5000));
        touch(dir.path(), "new.txt", Duration::from_secs(10));
        touch(dir.path(), "middle.txt", Duration::from_secs(300));

        assert_eq!(
            listed(dir.path()),
            ["Apple", "zebra", "new.txt", "middle.txt", "old.txt"]
        );
    }

    #[test]
    fn files_modified_together_fall_back_to_their_name() {
        let dir = tempfile::tempdir().unwrap();
        let moment = SystemTime::now() - Duration::from_secs(60);
        for name in ["b.txt", "A.txt", "c.txt"] {
            let file = File::create(dir.path().join(name)).unwrap();
            file.set_modified(moment).unwrap();
        }

        assert_eq!(listed(dir.path()), ["A.txt", "b.txt", "c.txt"]);
    }

    #[test]
    fn dot_files_and_dot_folders_are_left_out() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".cache")).unwrap();
        std::fs::create_dir(dir.path().join("shown")).unwrap();
        touch(dir.path(), ".hidden", Duration::ZERO);
        touch(dir.path(), "visible.txt", Duration::ZERO);

        assert_eq!(listed(dir.path()), ["shown", "visible.txt"]);
    }

    #[test]
    fn a_file_carries_its_size_and_a_folder_has_none() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("five.txt"), b"12345").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();

        let entries = read_folder(dir.path()).unwrap();

        assert_eq!(entries[0].kind, Kind::Folder);
        assert!(matches!(entries[1].kind, Kind::File { size: 5, .. }));
    }

    #[test]
    fn an_empty_folder_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(read_folder(dir.path()), Ok(Vec::new()));
    }

    #[test]
    fn an_error_names_the_folder_by_its_own_name_not_its_whole_path() {
        let err = read_folder(Path::new("/definitely/not/here/")).unwrap_err();

        assert!(err.starts_with("can't open here: "), "got: {err}");
        assert!(!err.contains("/definitely"), "got: {err}");
    }

    #[test]
    fn a_folder_with_no_name_of_its_own_is_named_by_its_path_in_an_error() {
        // A path ending in `..` has no final name to give.
        let err = read_folder(Path::new("/definitely/not/here/..")).unwrap_err();

        assert!(
            err.starts_with("can't open /definitely/not/here/..: "),
            "got: {err}"
        );
    }

    #[test]
    fn a_missing_folder_is_an_error_naming_it() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");

        let err = read_folder(&missing).unwrap_err();

        assert!(err.starts_with("can't open missing: "), "got: {err}");
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_is_an_error_naming_it_and_why() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(&locked).is_ok() {
            // Running as root: permissions don't apply, so nothing is refused.
            return;
        }

        let err = read_folder(&locked).unwrap_err();

        assert!(err.starts_with("can't open locked: "), "got: {err}");
        assert!(
            err.to_lowercase().contains("permission denied"),
            "got: {err}"
        );
        // So the temporary directory can be removed.
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_link_counts_as_what_it_points_to_and_a_broken_one_is_left_out() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("real_folder")).unwrap();
        touch(dir.path(), "real_file.txt", Duration::ZERO);
        symlink(
            dir.path().join("real_folder"),
            dir.path().join("link_to_folder"),
        )
        .unwrap();
        symlink(
            dir.path().join("real_file.txt"),
            dir.path().join("link_to_file"),
        )
        .unwrap();
        symlink(dir.path().join("nowhere"), dir.path().join("broken_link")).unwrap();

        let entries = read_folder(dir.path()).unwrap();

        let kind_of = |name: &str| {
            entries
                .iter()
                .find(|entry| entry.name == name)
                .map(|entry| entry.is_folder())
        };
        assert_eq!(kind_of("link_to_folder"), Some(true));
        assert_eq!(kind_of("link_to_file"), Some(false));
        assert_eq!(kind_of("broken_link"), None);
    }

    // -- the picker's state --------------------------------------------------

    #[test]
    fn a_new_picker_is_reading_and_shows_nothing_yet() {
        let picker = FilePicker::new("general".to_string(), PathBuf::from("/pics"), 1);

        assert!(picker.is_reading());
        assert_eq!(picker.dir(), Path::new("/pics"));
        assert_eq!(picker.visible().len(), 0);
    }

    #[test]
    fn a_listing_shows_the_entries_and_highlights_the_first() {
        let picker = pics();

        assert!(!picker.is_reading());
        assert_eq!(shown(&picker), ["docs", "a.txt", "b.txt", "c.txt"]);
        assert_eq!(picker.selected(), 0);
    }

    #[test]
    fn an_answer_to_an_earlier_read_is_ignored() {
        let mut picker = FilePicker::new("general".to_string(), PathBuf::from("/pics"), 1);
        picker.read(2, PathBuf::from("/pics/docs"));

        picker.apply(FolderListing {
            request: 1,
            result: Ok(vec![file("stale.txt")]),
        });
        assert!(picker.is_reading(), "still waiting for the second read");
        assert_eq!(picker.visible().len(), 0);

        picker.apply(FolderListing {
            request: 2,
            result: Ok(vec![file("fresh.txt")]),
        });
        assert_eq!(picker.dir(), Path::new("/pics/docs"));
        assert_eq!(shown(&picker), ["fresh.txt"]);
    }

    #[test]
    fn an_answer_nobody_is_waiting_for_is_ignored() {
        let mut picker = pics();

        picker.apply(FolderListing {
            request: 1,
            result: Ok(vec![file("again.txt")]),
        });

        assert_eq!(shown(&picker), ["docs", "a.txt", "b.txt", "c.txt"]);
    }

    #[test]
    fn a_folder_that_cannot_be_read_keeps_the_old_listing_and_says_why() {
        let mut picker = pics();
        picker.read(2, PathBuf::from("/pics/secret"));

        picker.apply(FolderListing {
            request: 2,
            result: Err("can't open /pics/secret: Permission denied".to_string()),
        });

        assert_eq!(picker.dir(), Path::new("/pics"));
        assert_eq!(shown(&picker), ["docs", "a.txt", "b.txt", "c.txt"]);
        assert_eq!(
            picker.notice(),
            Some("can't open /pics/secret: Permission denied")
        );
        assert!(!picker.is_reading());
    }

    #[test]
    fn a_notice_lasts_until_the_next_key_press() {
        let mut picker = pics();
        picker.read(2, PathBuf::from("/pics/secret"));
        picker.apply(FolderListing {
            request: 2,
            result: Err("can't open /pics/secret: Permission denied".to_string()),
        });

        picker.handle_key(key(KeyCode::Down));

        assert_eq!(picker.notice(), None);
    }

    #[test]
    fn what_was_typed_while_the_first_listing_was_awaited_filters_it_when_it_arrives() {
        let mut picker = FilePicker::new("general".to_string(), PathBuf::from("/pics"), 1);
        typed(&mut picker, "B.T");

        picker.apply(FolderListing {
            request: 1,
            result: Ok(vec![file("a.txt"), file("b.txt"), file("c.md")]),
        });

        assert_eq!(picker.query(), "B.T");
        assert_eq!(shown(&picker), ["b.txt"]);
    }

    #[test]
    fn a_new_folder_clears_the_filter_and_highlights_its_first_row() {
        let mut picker = pics();
        typed(&mut picker, "txt");
        picker.read(2, PathBuf::from("/pics/docs"));

        picker.apply(FolderListing {
            request: 2,
            result: Ok(vec![file("x.md"), file("y.md")]),
        });

        assert_eq!(picker.query(), "");
        assert_eq!(shown(&picker), ["x.md", "y.md"]);
        assert_eq!(picker.selected(), 0);
    }

    #[test]
    fn typing_narrows_the_folder_ignoring_case_and_highlights_the_first_match() {
        let mut picker = picker_on(
            "/pics",
            vec![folder("Docs"), file("Report.PDF"), file("notes.txt")],
        );
        picker.handle_key(key(KeyCode::Down));

        typed(&mut picker, "DOC");

        assert_eq!(shown(&picker), ["Docs"]);
        assert_eq!(picker.selected(), 0);
        typed(&mut picker, "s");
        assert_eq!(shown(&picker), ["Docs"]);
    }

    #[test]
    fn a_filter_that_matches_nothing_shows_nothing_and_enter_does_nothing() {
        let mut picker = pics();

        typed(&mut picker, "zzz");

        assert_eq!(shown(&picker), Vec::<&str>::new());
        assert_eq!(highlighted(&picker), None);
        assert_eq!(picker.handle_key(key(KeyCode::Enter)), Step::Stay);
    }

    #[test]
    fn a_pasted_filter_narrows_the_folder_too() {
        let mut picker = picker_on("/pics", vec![file("Report.PDF"), file("notes.txt")]);

        picker.edit_query(|query| query.push_str("rep"));

        assert_eq!(shown(&picker), ["Report.PDF"]);
    }

    #[test]
    fn backspace_deletes_a_typed_character_before_it_goes_up() {
        let mut picker = picker_on("/pics/docs", vec![file("a.txt"), file("b.txt")]);
        typed(&mut picker, "a");

        assert_eq!(picker.handle_key(key(KeyCode::Backspace)), Step::Stay);
        assert_eq!(picker.query(), "");
        assert_eq!(shown(&picker), ["a.txt", "b.txt"]);

        assert_eq!(
            picker.handle_key(key(KeyCode::Backspace)),
            Step::Open(PathBuf::from("/pics"))
        );
    }

    #[test]
    fn backspace_never_goes_up_past_the_root() {
        let mut picker = picker_on("/", vec![folder("usr")]);

        assert_eq!(picker.handle_key(key(KeyCode::Backspace)), Step::Stay);
    }

    #[test]
    fn going_up_highlights_the_folder_just_left() {
        let mut picker = picker_on("/pics/docs", vec![file("a.txt")]);
        let Step::Open(parent) = picker.handle_key(key(KeyCode::Backspace)) else {
            panic!("expected to go up");
        };

        picker.read(2, parent);
        picker.apply(FolderListing {
            request: 2,
            result: Ok(vec![folder("archive"), folder("docs"), folder("music")]),
        });

        assert_eq!(picker.dir(), Path::new("/pics"));
        assert_eq!(highlighted(&picker), Some("docs"));
    }

    #[test]
    fn going_into_a_folder_highlights_its_first_row() {
        let mut picker = pics();
        let Step::Open(docs) = picker.handle_key(key(KeyCode::Enter)) else {
            panic!("expected to open the folder");
        };

        picker.read(2, docs);
        picker.apply(FolderListing {
            request: 2,
            result: Ok(vec![file("one.txt"), file("two.txt")]),
        });

        assert_eq!(highlighted(&picker), Some("one.txt"));
    }

    #[test]
    fn enter_opens_a_folder() {
        let mut picker = pics();

        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            Step::Open(PathBuf::from("/pics/docs"))
        );
    }

    #[test]
    fn enter_on_a_file_chooses_it_when_nothing_is_marked() {
        let mut picker = pics();
        picker.handle_key(key(KeyCode::Down));
        picker.handle_key(key(KeyCode::Down));

        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            Step::Choose(paths(&["b.txt"]))
        );
    }

    #[test]
    fn space_marks_and_unmarks_the_highlighted_file() {
        let mut picker = pics();
        picker.handle_key(key(KeyCode::Down));
        let a = file("a.txt");

        picker.handle_key(key(KeyCode::Char(' ')));
        assert!(picker.is_marked(&a));
        assert_eq!(picker.marked_count(), 1);

        picker.handle_key(key(KeyCode::Char(' ')));
        assert!(!picker.is_marked(&a));
        assert_eq!(picker.marked_count(), 0);
    }

    #[test]
    fn space_on_a_folder_explains_instead_of_marking() {
        let mut picker = pics();

        picker.handle_key(key(KeyCode::Char(' ')));

        assert_eq!(picker.marked_count(), 0);
        assert!(
            picker
                .notice()
                .is_some_and(|notice| notice.contains("only files"))
        );
    }

    #[test]
    fn space_is_a_mark_not_a_filter_character() {
        let mut picker = pics();
        picker.handle_key(key(KeyCode::Down));

        picker.handle_key(key(KeyCode::Char(' ')));

        assert_eq!(picker.query(), "");
    }

    #[test]
    fn enter_chooses_the_marked_files_in_the_order_they_were_marked() {
        let mut picker = pics();
        for _ in 0..3 {
            picker.handle_key(key(KeyCode::Down)); // c.txt
        }
        picker.handle_key(key(KeyCode::Char(' ')));
        picker.handle_key(key(KeyCode::Up)); // b.txt
        picker.handle_key(key(KeyCode::Up)); // a.txt
        picker.handle_key(key(KeyCode::Char(' ')));

        // Enter on an unmarked file still chooses the marked ones only.
        picker.handle_key(key(KeyCode::Down)); // b.txt, unmarked
        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            Step::Choose(paths(&["c.txt", "a.txt"]))
        );
    }

    #[test]
    fn enter_on_a_folder_opens_it_even_with_files_marked() {
        let mut picker = pics();
        picker.handle_key(key(KeyCode::Down));
        picker.handle_key(key(KeyCode::Char(' ')));
        picker.handle_key(key(KeyCode::Up)); // docs

        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            Step::Open(PathBuf::from("/pics/docs"))
        );
    }

    #[test]
    fn marks_survive_changing_folder() {
        let mut picker = pics();
        picker.handle_key(key(KeyCode::Down));
        picker.handle_key(key(KeyCode::Char(' '))); // /pics/a.txt
        picker.read(2, PathBuf::from("/pics/docs"));
        picker.apply(FolderListing {
            request: 2,
            result: Ok(vec![file("a.txt"), file("z.txt")]),
        });

        // /pics/docs/a.txt is a different file from the marked /pics/a.txt.
        assert!(!picker.is_marked(&file("a.txt")));
        assert_eq!(picker.marked_count(), 1);
        picker.handle_key(key(KeyCode::Char(' '))); // mark /pics/docs/a.txt too
        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            Step::Choose(vec![
                PathBuf::from("/pics/a.txt"),
                PathBuf::from("/pics/docs/a.txt")
            ])
        );
    }

    #[test]
    fn marked_files_can_still_be_chosen_when_the_filter_shows_nothing() {
        let mut picker = pics();
        picker.handle_key(key(KeyCode::Down));
        picker.handle_key(key(KeyCode::Char(' ')));
        typed(&mut picker, "zzz");

        assert_eq!(
            picker.handle_key(key(KeyCode::Enter)),
            Step::Choose(paths(&["a.txt"]))
        );
    }

    #[test]
    fn up_and_down_move_the_highlight_and_stop_at_the_ends() {
        let mut picker = pics();

        picker.handle_key(key(KeyCode::Up));
        assert_eq!(picker.selected(), 0, "can't go above the first row");

        for _ in 0..10 {
            picker.handle_key(key(KeyCode::Down));
        }
        assert_eq!(picker.selected(), 3, "can't go below the last row");
        assert_eq!(highlighted(&picker), Some("c.txt"));
    }

    #[test]
    fn moving_in_an_empty_folder_is_harmless() {
        let mut picker = picker_on("/pics", Vec::new());

        picker.handle_key(key(KeyCode::Down));
        picker.handle_key(key(KeyCode::Up));

        assert_eq!(picker.selected(), 0);
        assert!(picker.is_empty());
    }

    #[test]
    fn escape_cancels() {
        let mut picker = pics();

        assert_eq!(picker.handle_key(key(KeyCode::Esc)), Step::Cancel);
    }

    #[test]
    fn control_chords_and_other_keys_are_not_typed() {
        let mut picker = pics();

        picker.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        picker.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::CONTROL));
        picker.handle_key(key(KeyCode::Tab));
        picker.handle_key(key(KeyCode::Left));

        assert_eq!(picker.query(), "");
        assert_eq!(picker.marked_count(), 0);
    }

    // -- the folder it opens in ----------------------------------------------

    #[test]
    fn the_first_candidate_that_is_a_folder_wins() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = dir.path().join("a_file");
        std::fs::write(&file, b"x").unwrap();

        let chosen = first_folder([
            None,
            Some(dir.path().join("missing")),
            Some(file),
            Some(dir.path().to_path_buf()),
            Some(other.path().to_path_buf()),
        ]);

        assert_eq!(chosen, dir.path());
    }

    #[test]
    fn with_no_candidate_it_opens_at_the_root() {
        assert_eq!(
            first_folder([None, Some(PathBuf::from("/definitely/not/here"))]),
            PathBuf::from(std::path::MAIN_SEPARATOR_STR)
        );
    }

    #[test]
    fn a_remembered_folder_round_trips() {
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();

        remember_folder(data.path(), folder.path()).unwrap();

        assert_eq!(
            remembered_folder(data.path()),
            Some(folder.path().to_path_buf())
        );
        assert_eq!(start_folder(data.path()), folder.path());
    }

    #[test]
    fn a_later_folder_replaces_the_remembered_one() {
        let data = tempfile::tempdir().unwrap();
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();

        remember_folder(data.path(), first.path()).unwrap();
        remember_folder(data.path(), second.path()).unwrap();

        assert_eq!(start_folder(data.path()), second.path());
    }

    #[test]
    fn nothing_remembered_is_none() {
        let data = tempfile::tempdir().unwrap();

        assert_eq!(remembered_folder(data.path()), None);
        assert!(start_folder(data.path()).is_dir());
    }

    #[test]
    fn a_remembered_folder_that_is_gone_is_not_used() {
        let data = tempfile::tempdir().unwrap();
        let gone = data.path().join("gone");
        remember_folder(data.path(), &gone).unwrap();

        let start = start_folder(data.path());

        assert_ne!(start, gone);
        assert!(start.is_dir());
    }

    #[test]
    fn remembering_creates_the_data_directory_if_needed() {
        let parent = tempfile::tempdir().unwrap();
        let data = parent.path().join("not").join("yet");

        remember_folder(&data, parent.path()).unwrap();

        assert_eq!(remembered_folder(&data), Some(parent.path().to_path_buf()));
    }
}
