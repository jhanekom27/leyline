//! Local filesystem policy for shared files -- see features.md's "File
//! sharing" idea. Kept separate from backfill.rs (which only knows about
//! content-addressed blobs, not real paths) and net.rs (transport): this
//! is where a peer's announced filename turns into an actual, safe path on
//! disk -- in a folder of its own channel's (`channel_dir`) under the one
//! files root (`files_root`) -- where "where did it go" gets answered with
//! zero configuration, and -- on the way out -- where a file someone wants
//! to share is checked before it is staged (`stage_file`/`stage_image`),
//! where a pasted path is recognized as a drag-and-drop (`dropped_files`),
//! and where a saved file is handed to the OS to open (`open_in_default_app`).

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

mod drops;
pub(crate) use drops::dropped_files;
#[cfg(test)]
use drops::*;
use std::process::{Command, Stdio};

use anyhow::Context;
use tracing::warn;

/// Subfolder created under the OS Downloads directory (or the fallback
/// below) so everything leyline has ever saved lives in one place instead
/// of cluttering the top-level Downloads folder.
const SUBDIR: &str = "leyline";

/// File in leyline's data directory that remembers where received files go
/// -- see `files_root`. A one-line text file of its own, deliberately not a
/// field of the postcard `settings` file, for the reason picker.rs's
/// `REMEMBERED_FOLDER` gives: postcard isn't self-describing, so a longer
/// struct would fail to decode an existing file and silently reset the
/// nickname and bell setting saved in it.
const FILES_ROOT_FILE: &str = "files_root";

/// File in leyline's data directory that remembers what `/maxsize` last set
/// -- see `size_limit`. A one-line text file of its own, for the same reason
/// as `FILES_ROOT_FILE`.
const SIZE_LIMIT_FILE: &str = "size_limit";

/// The longest channel name used as a folder name just as it is, and the
/// longest readable part of any other -- see `channel_folder`.
const MAX_FOLDER_NAME_CHARS: usize = 48;

/// How many hex digits of the hash of its name a channel folder carries when
/// the name isn't safe as it is: 64 bits, so that two names ending up in one
/// folder is out of the question -- by accident, and by design too, since a
/// channel's name can come from whoever shared its ticket.
const FOLDER_HASH_HEX_DIGITS: usize = 16;

/// How many "name (n).ext" candidates to try before giving up -- a safety
/// valve against a pathological loop, not a realistic limit.
const MAX_COLLISION_ATTEMPTS: u32 = 1000;

/// Largest file (or pasted image) that can be shared unless `/maxsize` says
/// otherwise (`SizeLimit`): a guardrail against an accidental huge share
/// (e.g. a mistyped path to a large video file). `BackfillStore::add_file`
/// copies the file into the local blob store, so a shared file's bytes are
/// duplicated on disk locally.
pub const DEFAULT_MAX_FILE_SIZE: u64 = 500 * 1024 * 1024;

/// How a save came about, which decides how far a file is trusted to be what
/// its sender said it was, and where the outcome is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveMode {
    /// Asked for by a person -- `/save`, or `Enter` in the files overlay --
    /// who has the file's name and size in front of them. Nothing limits its
    /// size, and the outcome is said in the channel they are in.
    Manual,
    /// Auto-save: nobody has looked at this file. It is refused above
    /// `max_bytes` of what it *is* (`BackfillStore::fetch_file` checks the
    /// verified size, never the one its sender announced), and the outcome is
    /// said in the channel it arrived in, whichever one the user is looking at.
    Automatic { max_bytes: u64 },
}

/// Where received files go until `set_files_root` says otherwise: the OS
/// Downloads folder (via `directories::UserDirs`) under a `leyline/`
/// subfolder, so a saved file ends up somewhere the user would actually
/// think to look, with zero configuration. Falls back to `fallback_base`
/// (leyline's own data directory) if no Downloads folder can be resolved at
/// all -- e.g. no resolvable home directory -- so saving always has
/// somewhere to go.
fn downloads_dir(fallback_base: &Path) -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(|dir| dir.join(SUBDIR)))
        .unwrap_or_else(|| fallback_base.join("downloads"))
}

/// The files root: the one folder every channel's folder lives in
/// (`channel_dir`). What `set_files_root` last recorded in `data_dir`, else
/// `downloads_dir` -- the `leyline` subfolder of the OS Downloads folder,
/// which is where every saved file went before channels had folders.
///
/// Read like `settings::Settings::load` reads its file: no file is simply
/// the default, and content that isn't an absolute path (hand-edited, or cut
/// short by a crash mid-write) is logged and ignored -- but a genuine I/O
/// error (permission denied, say) is returned, rather than quietly sending
/// files somewhere the user didn't choose.
pub fn files_root(data_dir: &Path) -> anyhow::Result<PathBuf> {
    let path = data_dir.join(FILES_ROOT_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(downloads_dir(data_dir));
        }
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    let root = PathBuf::from(text.trim_end_matches('\n'));
    if root.is_absolute() {
        Ok(root)
    } else {
        warn!(
            "ignoring {}: {text:?} is not an absolute path",
            path.display()
        );
        Ok(downloads_dir(data_dir))
    }
}

/// Moves the files root to `typed` -- as typed after `/folder `, so `~` and
/// a relative path are expanded (`expand_path`) -- and remembers it in
/// `data_dir`. The folder is created now, so a path that can't work (under
/// a plain file, no permission) is refused here, with the reason, instead
/// of failing the next `/save`. Files already saved stay where they are.
/// Returns the root as it now stands, for the caller to apply; the message
/// is for the user.
pub fn set_files_root(data_dir: &Path, typed: &str) -> Result<PathBuf, String> {
    let root = expand_path(typed);
    let text = root
        .to_str()
        .ok_or_else(|| format!("can't use {}: its name isn't valid UTF-8", root.display()))?;
    create_folder(&root).map_err(|err| format!("can't use {}: {err}", root.display()))?;
    std::fs::write(data_dir.join(FILES_ROOT_FILE), text).map_err(|err| {
        format!(
            "can't remember {} as the files folder: {err}",
            root.display()
        )
    })?;
    Ok(root)
}

/// The folder inside `root` that holds `channel`'s saved files -- `channel`
/// being its stored name (`app::Channel::name`), never the label shown for
/// it, so `/rename` and `/alias` can't move the folder. Not created here:
/// `resolve_destination` makes it when a file is saved into it.
pub fn channel_dir(root: &Path, channel: &str) -> PathBuf {
    root.join(channel_folder(channel))
}

/// The one-way drop folder for `channel`. It deliberately lives beside that
/// channel's received files, while remaining a local convention only: no
/// folder contents are synchronized or ever copied back from a peer.
pub fn outbox_dir(root: &Path, channel: &str) -> PathBuf {
    channel_dir(root, channel).join("send")
}

/// The name of the folder for the channel stored as `channel`: a pure
/// function of that name, so there is nothing to keep in step with it.
/// Nothing here can be trusted as a path -- `/join` takes any text, and a
/// ticket brings a name chosen by whoever shared it -- and no two channels
/// may share a folder, even where `General` and `general` are one name.
///
/// A name that is already a safe folder name everywhere -- lowercase ASCII
/// letters, digits, `-` and `_`, starting with a letter or digit, not too
/// long, and not one Windows reserves for a device -- is the folder
/// itself: `general`, `project-x`, `dm-07070707`. Anything else becomes a
/// readable approximation, a `~`, and a hash of the exact name
/// (`Team-Alpha~83f57e5b1e8db868`). A `~` never occurs in a plain name, so
/// a hashed folder can't be a plain one; and two different names hash
/// differently, which also keeps apart those that differ only by case,
/// Unicode form, punctuation, or whatever lies past the cut. What reaches
/// the filesystem is only letters, digits, `-`, `_` and `~`, never longer
/// than 48 characters + 17: no separators, `..`, characters Windows
/// refuses, control or direction marks, trailing dots, or device names.
///
/// Frozen once folders exist: changing this would strand them.
fn channel_folder(channel: &str) -> String {
    if is_plain_folder_name(channel) {
        return channel.to_string();
    }
    let hash = blake3::hash(channel.as_bytes()).to_hex();
    format!(
        "{}~{}",
        readable_part(channel),
        &hash[..FOLDER_HASH_HEX_DIGITS]
    )
}

/// Whether `name` can be a folder name as it is -- see `channel_folder`.
fn is_plain_folder_name(name: &str) -> bool {
    let plain_char = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    name.starts_with(plain_char)
        && name.len() <= MAX_FOLDER_NAME_CHARS
        && name
            .chars()
            .all(|c| plain_char(c) || matches!(c, '-' | '_'))
        && !is_windows_device_name(name)
}

/// Whether Windows reserves `name` (already lowercase) for a device, which
/// makes it impossible to create a folder of that name there.
fn is_windows_device_name(name: &str) -> bool {
    let numbered = |prefix: &str| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| matches!(rest.as_bytes(), [b'0'..=b'9']))
    };
    matches!(name, "con" | "prn" | "aux" | "nul") || numbered("com") || numbered("lpt")
}

/// The part of a hashed channel folder's name that a person can read: the
/// name's letters and digits (of any script) and `_`, with every run of
/// anything else squeezed into one `-`, cut to `MAX_FOLDER_NAME_CHARS`.
/// `channel` if nothing is left -- a name of nothing but punctuation or
/// emoji. Purely for the eye: the hash is what keeps folders apart.
fn readable_part(name: &str) -> String {
    let words: Vec<&str> = name
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
        .collect();
    let readable: String = words
        .join("-")
        .chars()
        .take(MAX_FOLDER_NAME_CHARS)
        .collect();
    match readable.trim_end_matches('-') {
        "" => "channel".to_string(),
        readable => readable.to_string(),
    }
}

/// Reduces a peer-announced filename to a single, safe path component:
/// just its final segment, with any leading path separators or `..`
/// components stripped. A peer's filename is untrusted, remote-controlled
/// data -- this is what stops it from ever escaping the destination
/// directory it's joined onto, the same boundary storage.rs's blake3-named
/// log files apply to channel names. Falls back to a generic name if
/// nothing safe is left (e.g. an empty string, or one made entirely of
/// path separators).
pub fn sanitize_filename(name: &str) -> String {
    match Path::new(name).file_name().and_then(|n| n.to_str()) {
        Some(safe) if !safe.is_empty() => safe.to_string(),
        _ => "file".to_string(),
    }
}

/// Creates `dir` and any of its parents that are missing. The one failure
/// worth a clearer message than the raw OS error is `dir`, or one of its
/// parents, already being a plain file rather than a folder -- e.g. a
/// `leyline` binary sitting directly in Downloads, colliding with the
/// `leyline/` folder the files root defaults to -- so `create_dir_all` can
/// never succeed there, however many times it is retried. Confirmed in the
/// wild: that is exactly what "File exists (os error 17)" meant. A parent
/// being the file is reported by the OS as "Not a directory" instead, which
/// says even less, so the message names the file in the way.
fn create_folder(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir).map_err(|err| match dir.ancestors().find(|path| path.is_file()) {
        Some(file) => std::io::Error::other(format!(
            "{} already exists and isn't a folder -- rename it, remove it, or move it elsewhere so leyline can save files there",
            file.display()
        )),
        None => err,
    })
}

/// Resolves a collision-free destination for `filename` inside `dir`,
/// creating `dir` if it doesn't exist yet. Never overwrites an existing
/// file and never prompts: if `filename` is already taken, tries
/// `name (1).ext`, `name (2).ext`, etc. -- the browser convention -- until
/// a free path is found.
pub fn resolve_destination(dir: &Path, filename: &str) -> std::io::Result<PathBuf> {
    create_folder(dir)?;

    let safe_name = sanitize_filename(filename);
    if let Some(path) = try_claim(dir, &safe_name)? {
        return Ok(path);
    }

    let name_path = Path::new(&safe_name);
    let stem = name_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(safe_name.as_str());
    let extension = name_path.extension().and_then(|e| e.to_str());

    for attempt in 1..=MAX_COLLISION_ATTEMPTS {
        let candidate_name = match extension {
            Some(ext) => format!("{stem} ({attempt}).{ext}"),
            None => format!("{stem} ({attempt})"),
        };
        if let Some(path) = try_claim(dir, &candidate_name)? {
            return Ok(path);
        }
    }
    Err(std::io::Error::other(format!(
        "could not find a free name for {safe_name:?} in {}",
        dir.display()
    )))
}

/// Atomically claims `dir.join(name)` as a destination by creating an
/// empty placeholder file there -- but only if nothing exists at that path
/// yet (`create_new`). Returns `None` (not an error) if the name is
/// already taken, so the caller moves on to the next candidate instead.
///
/// This closes a real race a plain "does it exist?" check leaves open:
/// two `/save`s resolving a destination at (nearly) the same time can
/// otherwise both see a name as free and both write to it, one silently
/// clobbering the other -- confirmed by racing concurrent saves of the
/// same file, which without this landed two different downloads on the
/// exact same path. The placeholder gets overwritten with the real bytes
/// once the download finishes (`net::Net::save_file`'s export), or
/// removed if the download fails instead (same place), so a failed save
/// never leaves a stray empty file behind. One tradeoff: since the
/// destination now always already exists as this empty placeholder,
/// iroh-blobs' reflink/copy-on-write fast path for the export never
/// applies (it requires the target to be absent) -- falling back to a
/// plain byte copy every time, which is a fine trade for closing the race.
fn try_claim(dir: &Path, name: &str) -> std::io::Result<Option<PathBuf>> {
    let path = dir.join(name);
    match OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(_) => Ok(Some(path)),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
        Err(err) => Err(err),
    }
}

/// The program that hands a file to whatever the OS has registered to open
/// it -- what double-clicking it would run: `open` on macOS, `explorer` on
/// Windows (which, given a file, opens it with its default application) and
/// `xdg-open` anywhere else.
#[cfg(target_os = "macos")]
const OPENER: &str = "open";
#[cfg(target_os = "windows")]
const OPENER: &str = "explorer";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const OPENER: &str = "xdg-open";

/// Opens `path` -- the absolute path of an existing file, so the launcher
/// can't take it for an option -- with the OS's default application for it,
/// as double-clicking it in a file manager would. Returns once the launcher
/// has been *started*: whether it then finds an application is the OS's
/// business and not something to wait for, so the only error is a launcher
/// that can't be run at all (no `xdg-open` on a bare server, say). The
/// launcher is reaped on a thread of its own, so one that outlives this
/// call is never left a zombie.
pub fn open_in_default_app(path: &Path) -> std::io::Result<()> {
    let mut launcher = open_command(path).spawn()?;
    std::thread::spawn(move || {
        let _ = launcher.wait();
    });
    Ok(())
}

/// The command `open_in_default_app` runs: the launcher with `path` as its
/// one argument -- an argument, never text for a shell, so nothing in a
/// file's name (which came from a peer) is ever interpreted -- and with
/// stdin, stdout and stderr closed off, so a launcher that prints can't draw
/// over the TUI.
fn open_command(path: &Path) -> Command {
    let mut command = Command::new(OPENER);
    command
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

/// The extensions (lowercase, no dot) of files that run, install or apply
/// something when the OS opens them: programs and what installs them,
/// scripts, and the shortcuts, launchers and settings files that send the OS
/// wherever their author chose. See `runs_when_opened`.
///
/// Best effort, and wider than any one system needs: a file's name is a
/// peer's to choose and says nothing about where it will be opened, so a
/// script that merely opens in an editor on one system is listed because
/// another runs it. It is not a sandbox -- what it doesn't name (a document
/// with macros, an archive, a web page) the OS opens as it always did.
const RUNS_WHEN_OPENED: &[&str] = &[
    // Programs, and what installs them.
    "exe",
    "com",
    "scr",
    "pif",
    "msi",
    "msp",
    "msix",
    "appx",
    "application",
    "jar",
    "jnlp",
    "pkg",
    "mpkg",
    "dmg",
    "deb",
    "rpm",
    "appimage",
    // Scripts.
    "bat",
    "cmd",
    "ps1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "py",
    "pyw",
    "pl",
    "rb",
    "sh",
    "bash",
    "zsh",
    "ksh",
    "csh",
    "command",
    "tool",
    // Shortcuts, launchers and settings that apply themselves.
    "lnk",
    "url",
    "webloc",
    "fileloc",
    "inetloc",
    "desktop",
    "terminal",
    "reg",
    "cpl",
    "chm",
    "mobileconfig",
];

/// Whether opening `path` -- a saved copy of a file, whose *name* is what the
/// OS goes by -- would run, install or apply something instead of just
/// showing it: its extension is on `RUNS_WHEN_OPENED`. `Ctrl+O` in the files
/// overlay refuses such a file, which is saved like any other but never
/// handed to the OS from there (`app::AppState::file_offer`).
///
/// Only the file's own name counts, not the folders above it, and only its
/// last extension, as the OS reads it: `report.pdf.exe` is a program and
/// `setup.exe.txt` is not. The name is read twice, as written and as
/// Windows reads it -- trailing dots and blanks dropped, and a `:` and all
/// after it taken for a stream of the file before it -- because Windows
/// opens `setup.exe.` and `setup.exe::$DATA` as `setup.exe`, and a peer can
/// pick either as a name that gets past a plain look at the extension.
pub fn runs_when_opened(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let as_windows_reads_it = name.split(':').next().unwrap_or_default();
    [&*name, as_windows_reads_it].into_iter().any(|reading| {
        reading
            .trim_end_matches(['.', ' '])
            .rsplit_once('.')
            .is_some_and(|(_, extension)| {
                RUNS_WHEN_OPENED.contains(&extension.to_ascii_lowercase().as_str())
            })
    })
}

/// A file, or a pasted image, waiting in a channel's staging area to be
/// shared -- see `app::Channel::staged`. Staging separates *choosing*
/// something to share (`Ctrl+V`/`/paste`) from *sending* it, so what is
/// about to go out is visible, can take a caption, and can still be
/// withdrawn. Always built by `stage_file`/`stage_image`, the one place an
/// outgoing file is checked, so everything downstream
/// (`net::Net::send_staged`) can trust it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedAttachment {
    /// The name it will be shared under.
    pub filename: String,
    /// Size in bytes as of staging -- what the strip above the input box
    /// shows, and what peers see before deciding whether to `/save`.
    pub size: u64,
    /// Where the content comes from once it is sent.
    pub source: StagedSource,
}

/// Where a `StagedAttachment`'s bytes come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagedSource {
    /// A file on disk, as an absolute path `stage_file` has already
    /// checked. Read when sent, not when staged, so what goes out is
    /// whatever the file holds at that moment.
    Path(PathBuf),
    /// Bytes with no file behind them: a clipboard image re-encoded as PNG
    /// (see `net::encode_png`), which only ever exists in memory.
    Bytes(Vec<u8>),
}

/// Expands a `/send`-typed path for the local filesystem: a leading `~`
/// (bare, or `~/...`) to the user's home directory, and any relative path
/// to absolute. Both matter because the path is typed directly into
/// leyline's own input box, not a shell -- nothing else expands `~`
/// here -- and because iroh-blobs' importer (`BackfillStore::add_file`)
/// hard-requires an absolute path, rejecting a relative one outright.
/// Falls back to the path as typed if the home or current directory can't
/// be determined.
pub fn expand_path(path: &str) -> PathBuf {
    let expanded = if path == "~" {
        directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
    } else if let Some(rest) = path.strip_prefix("~/") {
        directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(rest))
    } else {
        None
    }
    .unwrap_or_else(|| PathBuf::from(path));

    std::path::absolute(&expanded).unwrap_or(expanded)
}

/// How big a file (or pasted image) may be to share: the guardrail against
/// an accidental huge share, which `/maxsize` moves or lifts for a deliberate
/// one. The three variants are exactly what is remembered across restarts
/// (`size_limit`, `set_size_limit`) -- nothing remembered is `Default`, so a
/// build that changes the default moves everyone who never chose a limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeLimit {
    /// `DEFAULT_MAX_FILE_SIZE`.
    Default,
    /// At most this many bytes -- at least one, since a limit of none would
    /// refuse everything (`SizeLimit::parse`).
    Bytes(u64),
    /// No limit.
    Off,
}

impl SizeLimit {
    /// The most bytes a shared file may have, or `None` for no limit.
    pub fn max_bytes(self) -> Option<u64> {
        match self {
            SizeLimit::Default => Some(DEFAULT_MAX_FILE_SIZE),
            SizeLimit::Bytes(bytes) => Some(bytes),
            SizeLimit::Off => None,
        }
    }

    /// Reads a limit as typed after `/maxsize` -- and as remembered, which is
    /// the same text: `default`, `off`, or a size, a number and a unit (`B`,
    /// `KB`, `MB`, `GB` or `TB`, in any case, with or without a blank between)
    /// such as `750MB`, `1.5 GB` or `2gb`. The units are the ones
    /// `human_size` prints, 1024 apart, so what a refusal says is what can be
    /// typed. The one place a limit is validated, so everything downstream
    /// can trust it: a size with no unit, an unknown unit or a malformed
    /// number is refused, and so is one that rounds down to nothing (it would
    /// refuse every file) or is too big to count (`off` lifts the limit).
    pub fn parse(text: &str) -> Result<SizeLimit, String> {
        let text = text.trim();
        match text.to_ascii_lowercase().as_str() {
            "default" => return Ok(SizeLimit::Default),
            "off" => return Ok(SizeLimit::Off),
            _ => {}
        }
        let malformed = || {
            format!(
                "can't read '{text}' as a size -- give a number and a unit (B, KB, MB, GB or TB), like 750MB or 2GB, or say off or default"
            )
        };
        let (number, unit) = text.split_at(
            text.find(|c: char| !(c.is_ascii_digit() || c == '.'))
                .unwrap_or(text.len()),
        );
        let unit: u64 = match unit.trim().to_ascii_uppercase().as_str() {
            "B" => 1,
            "KB" => 1 << 10,
            "MB" => 1 << 20,
            "GB" => 1 << 30,
            "TB" => 1 << 40,
            _ => return Err(malformed()),
        };
        let number: f64 = number.parse().map_err(|_| malformed())?;
        let bytes = number * unit as f64;
        if bytes < 1.0 {
            return Err(format!(
                "a limit of '{text}' would refuse every file -- /maxsize <size> sets a real one, /maxsize off lifts the limit"
            ));
        }
        if bytes >= u64::MAX as f64 {
            return Err(format!(
                "'{text}' is too big to count -- /maxsize off lifts the limit instead"
            ));
        }
        Ok(SizeLimit::Bytes(bytes as u64))
    }
}

/// The size limit `set_size_limit` last recorded in `data_dir`: nothing
/// recorded is `Default`. Read like `files_root` reads its file -- content
/// that isn't a limit (hand-edited, or cut short by a crash mid-write) is
/// logged and ignored, which means the default: a guarded state, never a
/// looser one -- but a genuine I/O error (permission denied, say) is
/// returned, rather than quietly applying a limit nobody chose.
pub fn size_limit(data_dir: &Path) -> anyhow::Result<SizeLimit> {
    let path = data_dir.join(SIZE_LIMIT_FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(SizeLimit::Default);
        }
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    Ok(SizeLimit::parse(&text).unwrap_or_else(|reason| {
        warn!("ignoring {}: {reason}", path.display());
        SizeLimit::Default
    }))
}

/// Remembers `limit` in `data_dir` for the next start -- `Default` by
/// forgetting whatever was remembered, so that nothing is. The message is
/// for the user.
pub fn set_size_limit(data_dir: &Path, limit: SizeLimit) -> Result<(), String> {
    let path = data_dir.join(SIZE_LIMIT_FILE);
    let recorded = match limit {
        SizeLimit::Default => match std::fs::remove_file(&path) {
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
        SizeLimit::Bytes(bytes) => std::fs::write(&path, format!("{bytes}B")),
        SizeLimit::Off => std::fs::write(&path, "off"),
    };
    recorded.map_err(|err| format!("can't remember the size limit: {err}"))
}

/// Checks `path` (already expanded, see `expand_path`) for sharing and
/// stages it: it must exist, be a file rather than a directory, be
/// readable, fit within `limit`, and have a name to share it under.
/// Refusals read like the ones `/send` has always given, since `/send` and
/// the clipboard both stage through here -- one boundary, so what each of
/// them refuses and how it says so cannot drift apart.
pub fn stage_file(path: &Path, limit: SizeLimit) -> Result<StagedAttachment, String> {
    let metadata =
        std::fs::metadata(path).map_err(|err| format!("can't send {}: {err}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("can't send {}: not a file", path.display()));
    }
    // `metadata` works on a file whose contents can't be read (no read
    // permission), so try opening it: refusing now beats failing at import
    // time, after a caption has been typed.
    std::fs::File::open(path).map_err(|err| format!("can't send {}: {err}", path.display()))?;
    if let Some(reason) = too_large(metadata.len(), limit) {
        return Err(format!("can't send {}: {reason}", path.display()));
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| format!("can't send {}: can't determine a filename", path.display()))?;
    Ok(StagedAttachment {
        filename,
        size: metadata.len(),
        source: StagedSource::Path(path.to_path_buf()),
    })
}

/// Stages already-in-hand bytes (a clipboard image re-encoded as PNG, see
/// `net::Net::paste`) under `filename`, subject to the same `limit` as
/// `stage_file` -- there is no path to inspect, so the size is the only
/// thing to check.
pub fn stage_image(
    filename: String,
    bytes: Vec<u8>,
    limit: SizeLimit,
) -> Result<StagedAttachment, String> {
    let size = bytes.len() as u64;
    if let Some(reason) = too_large(size, limit) {
        return Err(format!("can't paste image: {reason}"));
    }
    Ok(StagedAttachment {
        filename,
        size,
        source: StagedSource::Bytes(bytes),
    })
}

/// Why `size` bytes can't be shared under `limit`, or `None` if they can --
/// the wording shared by `stage_file`'s and `stage_image`'s refusals, and the
/// one place that says how to get past the limit.
fn too_large(size: u64, limit: SizeLimit) -> Option<String> {
    let max = limit.max_bytes().filter(|&max| size > max)?;
    Some(format!(
        "too large ({}, max {}) -- raise the limit with /maxsize <size> (say /maxsize 2GB) or lift it with /maxsize off",
        human_size(size),
        human_size(max)
    ))
}

/// The files a bracketed paste consists of, if it is nothing but existing
/// files -- which is how a drag-and-drop shows up in a terminal. There is
/// no drop protocol: the terminal pastes the dropped files' paths (and
/// never says where they were dropped, so the caller stages them on the
/// active channel). The spelling varies by terminal, and every common one
/// is read here: backslash-escaped (Terminal.app, iTerm2, Ghostty),
/// single-quoted (iTerm2's quoting option, GNOME Terminal), double-quoted
/// (WezTerm's Posix mode, Windows Terminal), `file://` URIs, several files
/// joined by spaces or newlines -- and kitty and older Alacritty paste a
/// path raw, spaces and all.
///
/// The decision is all or nothing, and the filesystem is its arbiter:
/// every path must be absolute and name an existing regular file, or the
/// whole paste is ordinary text. A sentence that mentions a path, a bare
/// word like `README.md` that happens to match a file in the working
/// directory, a directory path, and a path that no longer exists are all
/// left as text. A false positive would eat what the user pasted; a false
/// Formats a byte count for display, e.g. `2.1 MB`. Binary (1024-based)
/// units, labeled with the familiar decimal-looking suffixes rather than
/// KiB/MiB -- precision doesn't matter here, just a quick sense of scale.
pub fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_keeps_a_plain_name() {
        assert_eq!(sanitize_filename("report.pdf"), "report.pdf");
    }

    #[test]
    fn sanitize_filename_strips_directory_components() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("/etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("a/b/c.txt"), "c.txt");
    }

    #[test]
    fn sanitize_filename_falls_back_when_nothing_safe_is_left() {
        assert_eq!(sanitize_filename(""), "file");
        assert_eq!(sanitize_filename(".."), "file");
        assert_eq!(sanitize_filename("/"), "file");
    }

    #[test]
    fn resolve_destination_uses_the_plain_name_when_free() {
        let dir = tempfile::tempdir().unwrap();
        let path = resolve_destination(dir.path(), "report.pdf").unwrap();
        assert_eq!(path, dir.path().join("report.pdf"));
    }

    #[test]
    fn resolve_destination_avoids_a_collision_by_appending_a_counter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("report.pdf"), b"existing").unwrap();

        let path = resolve_destination(dir.path(), "report.pdf").unwrap();

        assert_eq!(path, dir.path().join("report (1).pdf"));
    }

    #[test]
    fn resolve_destination_keeps_incrementing_past_multiple_collisions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("report.pdf"), b"a").unwrap();
        std::fs::write(dir.path().join("report (1).pdf"), b"b").unwrap();

        let path = resolve_destination(dir.path(), "report.pdf").unwrap();

        assert_eq!(path, dir.path().join("report (2).pdf"));
    }

    #[test]
    fn resolve_destination_handles_a_name_with_no_extension() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README"), b"a").unwrap();

        let path = resolve_destination(dir.path(), "README").unwrap();

        assert_eq!(path, dir.path().join("README (1)"));
    }

    #[test]
    fn resolve_destination_creates_the_directory_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("nested");

        let path = resolve_destination(&nested, "report.pdf").unwrap();

        assert!(nested.is_dir());
        assert_eq!(path, nested.join("report.pdf"));
    }

    #[test]
    fn resolve_destination_sanitizes_a_traversal_attempt() {
        let dir = tempfile::tempdir().unwrap();

        let path = resolve_destination(dir.path(), "../../etc/passwd").unwrap();

        assert_eq!(path, dir.path().join("passwd"));
    }

    #[test]
    fn resolve_destination_gives_a_clear_error_when_the_folder_path_is_a_file() {
        // Reproduces a real report: a `leyline` binary already sitting
        // directly in Downloads collides with the `leyline/` subfolder
        // `downloads_dir` always asks for, so `create_dir_all` fails with
        // a bare "File exists (os error 17)" -- this should be replaced
        // with a message that actually says what's wrong and how to fix it.
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("leyline");
        std::fs::write(&blocked, b"not a folder").unwrap();

        let err = resolve_destination(&blocked, "report.pdf").unwrap_err();

        let message = err.to_string();
        assert!(
            message.contains(&blocked.display().to_string()),
            "got: {message}"
        );
        assert!(
            !message.contains("os error 17"),
            "should replace the cryptic OS error, got: {message}"
        );
    }

    #[test]
    fn resolve_destination_leaves_an_empty_placeholder_at_the_claimed_path() {
        // The whole point of claiming atomically: the returned path must
        // already exist (as an empty file) the moment resolve_destination
        // returns, not just "be free at the time of the check" -- that gap
        // is exactly what let two concurrent /save calls race onto the
        // same name before this.
        let dir = tempfile::tempdir().unwrap();
        let path = resolve_destination(dir.path(), "report.pdf").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn try_claim_prevents_a_second_concurrent_caller_from_getting_the_same_path() {
        let dir = tempfile::tempdir().unwrap();
        let first = try_claim(dir.path(), "report.pdf").unwrap();
        let second = try_claim(dir.path(), "report.pdf").unwrap();
        assert!(first.is_some());
        assert!(
            second.is_none(),
            "a second claim for the same name must not succeed while the first still holds it"
        );
    }

    #[test]
    fn resolve_destination_moves_past_a_claimed_but_still_empty_placeholder() {
        // Simulates a save already in flight (claimed, not yet written):
        // a second resolve_destination for the same filename must not
        // reuse it, even though it looks identical to a genuinely empty
        // downloaded file.
        let dir = tempfile::tempdir().unwrap();
        try_claim(dir.path(), "report.pdf").unwrap();

        let path = resolve_destination(dir.path(), "report.pdf").unwrap();

        assert_eq!(path, dir.path().join("report (1).pdf"));
    }

    #[test]
    fn resolve_destination_names_the_file_in_the_way_when_it_is_a_parent_of_the_folder() {
        // The same report, one level down: the channel folder sits below the
        // `leyline` file, so the OS says "Not a directory" -- and the message
        // has to name the file that is actually in the way.
        let dir = tempfile::tempdir().unwrap();
        let blocked = dir.path().join("leyline");
        std::fs::write(&blocked, b"not a folder").unwrap();

        let err = resolve_destination(&blocked.join("general"), "report.pdf").unwrap_err();

        let message = err.to_string();
        assert!(
            message.contains(&blocked.display().to_string()),
            "got: {message}"
        );
        assert!(
            !message.contains("os error"),
            "should replace the cryptic OS error, got: {message}"
        );
    }

    #[test]
    fn each_channel_saves_into_a_folder_of_its_own_created_on_demand() {
        let root = tempfile::tempdir().unwrap();
        let general = channel_dir(root.path(), "general");
        let project = channel_dir(root.path(), "project-x");
        assert!(!general.exists() && !project.exists());

        let first = resolve_destination(&project, "report.pdf").unwrap();
        // Collisions are still settled inside one channel's folder...
        let second = resolve_destination(&project, "report.pdf").unwrap();
        // ...and the same name in another channel's is no collision at all.
        let other = resolve_destination(&general, "report.pdf").unwrap();

        assert_eq!(first, root.path().join("project-x/report.pdf"));
        assert_eq!(second, root.path().join("project-x/report (1).pdf"));
        assert_eq!(other, root.path().join("general/report.pdf"));
    }

    #[test]
    fn a_name_that_is_safe_everywhere_is_its_own_folder() {
        for name in [
            "general",
            "project-x",
            "dm-07070707",
            "p2p-08080808",
            "snake_case-2",
            "0",
            "a",
        ] {
            assert_eq!(channel_folder(name), name);
        }
    }

    #[test]
    fn any_other_name_becomes_a_readable_part_and_the_hash_of_the_exact_name() {
        // Pinned, hashes included (blake3, checked against an independent
        // implementation): any change would strand the folders already made
        // under these names.
        assert_eq!(channel_folder("Team Alpha"), "Team-Alpha~83f57e5b1e8db868");
        assert_eq!(channel_folder("General"), "General~529526552c68187d");
        assert_eq!(channel_folder("dm-Alice"), "dm-Alice~65abe3107fac5dc4");
    }

    #[test]
    fn the_readable_part_keeps_letters_of_any_script_and_squeezes_the_rest() {
        let readable = |name: &str| channel_folder(name).split('~').next().unwrap().to_string();

        assert_eq!(readable("B\u{fc}ro & Co."), "B\u{fc}ro-Co");
        assert_eq!(
            readable("\u{65e5}\u{672c}\u{8a9e}"),
            "\u{65e5}\u{672c}\u{8a9e}"
        );
        assert_eq!(readable("  --my_project--  "), "my_project");
        assert_eq!(readable("a / b \\ c"), "a-b-c");
        for nothing_to_read in ["", "\u{1f3ae}", "...", "///"] {
            assert_eq!(readable(nothing_to_read), "channel", "{nothing_to_read:?}");
        }
    }

    #[test]
    fn a_safe_name_is_only_its_own_folder_up_to_a_length_limit() {
        let longest = "a".repeat(MAX_FOLDER_NAME_CHARS);
        assert_eq!(channel_folder(&longest), longest);

        let too_long = format!("{longest}a");
        assert!(channel_folder(&too_long).starts_with(&format!("{longest}~")));
    }

    #[test]
    fn a_name_windows_reserves_for_a_device_is_not_its_own_folder() {
        for name in [
            "con", "prn", "aux", "nul", "com0", "com1", "com9", "lpt0", "lpt9",
        ] {
            assert!(
                channel_folder(name).starts_with(&format!("{name}~")),
                "{name}"
            );
        }
        // Near misses are ordinary names.
        for name in ["console", "com", "com10", "auxiliary", "nul-1", "lpt"] {
            assert_eq!(channel_folder(name), name);
        }
    }

    /// Names that are trouble somewhere: a separator or a traversal, a
    /// character Windows refuses, a reserved device name in any case, a
    /// trailing dot or space, an invisible control or direction mark, a
    /// leading dash or tilde, something hidden, something enormous.
    fn awkward_names() -> Vec<String> {
        let mut names: Vec<String> = [
            "",
            ".",
            "..",
            "...",
            "/",
            "\\",
            "../../etc/passwd",
            "..\\..\\Windows\\System32",
            "a/b",
            "a\\b",
            "C:\\Users\\me",
            "/etc/passwd",
            "con",
            "CON",
            "Con",
            "nul",
            "AUX",
            "com1",
            "COM9",
            "lpt1",
            "LPT0",
            "nul.txt",
            "name.",
            "name ",
            " name",
            "name...",
            "a:b",
            "a<b>",
            "a|b",
            "a?b",
            "a*b",
            "a\"b",
            "tab\there",
            "new\nline",
            "nul\0byte",
            "\u{202e}evil\u{202c}",
            "zero\u{200b}width",
            "-rf",
            "~",
            "~root",
            "$HOME",
            "%APPDATA%",
            ".hidden",
            "thumbs.db",
            "\u{1f3ae}",
            "General",
            "\u{dc}n\u{ef}c\u{f6}d\u{e9}",
        ]
        .map(String::from)
        .to_vec();
        names.push("x".repeat(300));
        names.push("\u{e9}".repeat(300));
        names.push("\u{1d400}".repeat(300)); // a letter that takes 4 bytes
        names.push("\u{65e5}".repeat(300));
        names
    }

    #[test]
    fn a_channel_folder_is_safe_on_every_platform_whatever_the_name() {
        let devices: Vec<String> = ["con", "prn", "aux", "nul"]
            .into_iter()
            .map(String::from)
            .chain((0..=9).flat_map(|n| [format!("com{n}"), format!("lpt{n}")]))
            .collect();
        for name in awkward_names() {
            let folder = channel_folder(&name);
            let context = format!("{name:?} -> {folder:?}");

            // Only letters, digits, `-`, `_` and `~`: no separator, dot,
            // character Windows refuses, control or direction mark, or space.
            assert!(
                folder
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '~')),
                "{context}"
            );
            // Not an option to a shell command, nor a `~user` to expand.
            assert!(!folder.starts_with(['-', '~']), "{context}");
            // One ordinary path component: not `.`, `..` or a root.
            let mut components = Path::new(&folder).components();
            assert!(
                matches!(components.next(), Some(std::path::Component::Normal(_))),
                "{context}"
            );
            assert!(components.next().is_none(), "{context}");
            assert!(!devices.contains(&folder.to_lowercase()), "{context}");
            // Short enough for any filesystem's 255 bytes, as documented.
            assert!(folder.len() <= 255, "{context}");
            assert!(
                folder.chars().count() <= MAX_FOLDER_NAME_CHARS + 1 + FOLDER_HASH_HEX_DIGITS,
                "{context}"
            );
        }
    }

    #[test]
    fn different_channels_never_share_a_folder_even_where_case_is_ignored() {
        let long = "long".repeat(30);
        let names = [
            // Only the case differs: one name on macOS and Windows.
            "general".to_string(),
            "General".to_string(),
            "GENERAL".to_string(),
            "dm-alice".to_string(),
            "dm-Alice".to_string(),
            // Different characters, the same readable part.
            "a-b".to_string(),
            "a--b".to_string(),
            "a b".to_string(),
            "a/b".to_string(),
            "a?b".to_string(),
            "a.b".to_string(),
            "a:b".to_string(),
            // The same letter, as one character or as two.
            "caf\u{e9}".to_string(),
            "cafe\u{301}".to_string(),
            // The same start, cut at the same place.
            format!("{long}x"),
            format!("{long}y"),
            // A device name, in two cases.
            "con".to_string(),
            "CON".to_string(),
            // One made to look like the folder of another.
            "General~529526552c68187d".to_string(),
            // Nothing to read in any of these.
            String::new(),
            "\u{1f3ae}".to_string(),
            "\u{1f3b2}".to_string(),
        ];

        let mut folders = std::collections::HashSet::new();
        for name in &names {
            let folder = channel_folder(name).to_lowercase();
            assert!(
                folders.insert(folder.clone()),
                "{name:?} shares {folder:?} with another channel"
            );
        }
    }

    #[test]
    fn a_channel_folder_is_always_directly_under_the_root() {
        let root = Path::new("/files");

        assert_eq!(channel_dir(root, "general"), root.join("general"));
        for name in awkward_names() {
            assert_eq!(channel_dir(root, &name).parent(), Some(root), "{name:?}");
        }
    }

    #[test]
    fn the_files_root_is_the_downloads_folder_until_one_is_set() {
        let data = tempfile::tempdir().unwrap();

        assert_eq!(files_root(data.path()).unwrap(), downloads_dir(data.path()));
    }

    #[test]
    fn a_files_root_that_was_set_is_what_a_restart_reads_back() {
        let data = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let wanted = elsewhere.path().join("received");

        let root = set_files_root(data.path(), wanted.to_str().unwrap()).unwrap();

        assert_eq!(root, wanted);
        assert!(
            wanted.is_dir(),
            "made at once, so a bad path fails now and not at the next save"
        );
        assert_eq!(files_root(data.path()).unwrap(), wanted);
    }

    #[test]
    fn setting_the_files_root_again_replaces_it() {
        let data = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let first = elsewhere.path().join("first");
        let second = elsewhere.path().join("second");

        set_files_root(data.path(), first.to_str().unwrap()).unwrap();
        set_files_root(data.path(), second.to_str().unwrap()).unwrap();

        assert_eq!(files_root(data.path()).unwrap(), second);
    }

    #[test]
    fn a_files_root_setting_that_is_not_an_absolute_path_is_ignored() {
        let data = tempfile::tempdir().unwrap();
        for content in ["", "\n", "relative/dir", "~/Downloads"] {
            std::fs::write(data.path().join(FILES_ROOT_FILE), content).unwrap();

            assert_eq!(
                files_root(data.path()).unwrap(),
                downloads_dir(data.path()),
                "{content:?}"
            );
        }
    }

    #[test]
    fn a_files_root_setting_that_cannot_be_read_is_an_error_not_a_quiet_default() {
        let data = tempfile::tempdir().unwrap();
        // A folder where the file should be: not "missing", so not the default.
        std::fs::create_dir(data.path().join(FILES_ROOT_FILE)).unwrap();

        let err = files_root(data.path()).unwrap_err();

        assert!(err.to_string().contains(FILES_ROOT_FILE), "got: {err}");
    }

    #[test]
    fn set_files_root_refuses_a_path_under_a_file_and_remembers_nothing() {
        let data = tempfile::tempdir().unwrap();
        let blocker = data.path().join("not-a-folder");
        std::fs::write(&blocker, b"x").unwrap();
        let path = blocker.join("inside");

        let message = set_files_root(data.path(), path.to_str().unwrap()).unwrap_err();

        assert!(
            message.starts_with(&format!("can't use {}: ", path.display())),
            "got: {message}"
        );
        assert!(
            message.contains(&blocker.display().to_string()),
            "names the file in the way, got: {message}"
        );
        assert!(!data.path().join(FILES_ROOT_FILE).exists());
    }

    #[test]
    fn the_launcher_gets_the_path_as_its_one_argument_whatever_the_name_says() {
        // A name is a peer's to choose: shell syntax, a percent variable and
        // quotes in it must reach the launcher as plain characters.
        let path = Path::new("/files/general/a & b; $(echo hi) %PATH% \"q\".pdf");

        let command = open_command(path);

        assert_eq!(command.get_program(), OPENER);
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [path.as_os_str()],
            "one argument, the path as it is"
        );
    }

    #[test]
    fn a_launcher_that_cannot_be_run_is_an_error_not_a_panic() {
        // `Command::spawn` on a program that is not there, as on a server
        // with no `xdg-open`: reported to the caller, which tells the user.
        let mut missing = Command::new("leyline-no-such-launcher");

        assert!(missing.arg("/x").spawn().is_err());
    }

    /// `runs_when_opened` for a file called `name` in some channel folder.
    fn runs(name: &str) -> bool {
        runs_when_opened(&Path::new("/files/general").join(name))
    }

    #[test]
    fn programs_scripts_and_shortcuts_are_known_by_their_extension_whatever_its_case() {
        for name in [
            "setup.exe",
            "SETUP.EXE",
            "Setup.Exe",
            "installer.msi",
            "tool.jar",
            "run.bat",
            "run.cmd",
            "run.ps1",
            "run.vbs",
            "run.js",
            "run.py",
            "run.sh",
            "start.command",
            "app.pkg",
            "disk.dmg",
            "link.lnk",
            "page.url",
            "app.desktop",
            "settings.mobileconfig",
            "app.AppImage",
        ] {
            assert!(runs(name), "{name}");
        }
    }

    #[test]
    fn ordinary_files_are_left_alone() {
        for name in [
            "report.pdf",
            "photo.JPG",
            "notes.txt",
            "archive.zip",
            "song.mp3",
            "clip.mp4",
            "data.csv",
            "photo (1).png",
            "README",
            "Makefile",
            // The extension is what follows a dot, not any name that is one.
            "exe",
            "sh",
            "executable.txt",
            "a.b.c",
        ] {
            assert!(!runs(name), "{name}");
        }
        assert!(!runs_when_opened(Path::new("/")), "no name, nothing to run");
    }

    #[test]
    fn only_the_last_extension_counts() {
        assert!(runs("report.pdf.exe"), "a program dressed as a document");
        assert!(runs("archive.tar.sh"));
        assert!(!runs("setup.exe.txt"), "a text file with a misleading stem");
        assert!(!runs("notes.exe.pdf"));
    }

    #[test]
    fn a_dotfile_named_for_an_extension_still_counts() {
        assert!(runs(".exe"));
    }

    #[test]
    fn only_the_file_name_counts_not_the_folders_above_it() {
        assert!(!runs_when_opened(Path::new("/files/tools.exe/notes.txt")));
        assert!(runs_when_opened(Path::new("/files/docs.txt/setup.exe")));
    }

    #[test]
    fn the_names_windows_would_open_as_a_program_are_caught_too() {
        // Windows drops trailing dots and blanks, and takes a `:` for the
        // start of a stream of the file named before it.
        for name in [
            "setup.exe.",
            "setup.exe ",
            "setup.exe. .",
            "setup.exe::$DATA",
            "setup.exe:hidden",
            "SETUP.EXE::$data",
        ] {
            assert!(runs(name), "{name:?}");
        }
        // A `:` or a trailing dot alone doesn't make a document a program.
        for name in [
            "notes: part 1.txt",
            "2026-10-13T10:40:12.log",
            "report.pdf.",
        ] {
            assert!(!runs(name), "{name:?}");
        }
    }

    #[test]
    fn the_list_holds_only_extensions_that_can_match() {
        // `runs_when_opened` compares a lowercase extension with no dot, so
        // an entry that isn't exactly that can never match anything -- a typo
        // that would quietly let a kind of file through.
        for extension in RUNS_WHEN_OPENED {
            assert!(
                !extension.is_empty()
                    && extension
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "{extension:?}"
            );
        }
    }

    #[test]
    fn human_size_formats_bytes_and_larger_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(2_150_000), "2.1 MB");
    }

    fn home_dir() -> PathBuf {
        directories::BaseDirs::new()
            .unwrap()
            .home_dir()
            .to_path_buf()
    }

    #[test]
    fn expand_path_expands_a_bare_tilde_to_the_home_directory() {
        assert_eq!(expand_path("~"), home_dir());
    }

    #[test]
    fn expand_path_expands_a_tilde_prefixed_path() {
        assert_eq!(
            expand_path("~/Downloads/tilemap.png"),
            home_dir().join("Downloads/tilemap.png")
        );
    }

    #[test]
    fn expand_path_makes_a_relative_path_absolute() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(expand_path("README.md"), cwd.join("README.md"));
    }

    #[test]
    fn expand_path_leaves_an_already_absolute_path_alone() {
        assert_eq!(expand_path("/tmp/foo.txt"), PathBuf::from("/tmp/foo.txt"));
    }

    #[test]
    fn expand_path_does_not_expand_a_tilde_in_the_middle_of_a_path() {
        // Only a leading `~` is special, mirroring shell tilde expansion --
        // `foo/~/bar` is just a literal (if unusual) relative path.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(expand_path("foo/~/bar"), cwd.join("foo/~/bar"));
    }

    #[test]
    fn stage_file_reports_the_name_size_and_path_of_a_regular_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.pdf");
        std::fs::write(&path, b"twelve bytes").unwrap();

        let staged = stage_file(&path, SizeLimit::Default).unwrap();

        assert_eq!(
            staged,
            StagedAttachment {
                filename: "report.pdf".to_string(),
                size: 12,
                source: StagedSource::Path(path),
            }
        );
    }

    #[test]
    fn stage_file_refuses_a_missing_file_in_the_wording_send_uses() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.txt");

        let message = stage_file(&missing, SizeLimit::Default).unwrap_err();

        assert!(
            message.starts_with(&format!("can't send {}: ", missing.display())),
            "got: {message}"
        );
    }

    #[test]
    fn stage_file_refuses_a_directory() {
        let dir = tempfile::tempdir().unwrap();

        let message = stage_file(dir.path(), SizeLimit::Default).unwrap_err();

        assert_eq!(
            message,
            format!("can't send {}: not a file", dir.path().display())
        );
    }

    #[cfg(unix)]
    #[test]
    fn stage_file_refuses_a_file_it_cannot_read() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.txt");
        std::fs::write(&path, b"nope").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::File::open(&path).is_ok() {
            // Running as root: permissions don't apply, so there is
            // nothing to refuse.
            return;
        }

        let message = stage_file(&path, SizeLimit::Default).unwrap_err();

        assert!(
            message.starts_with(&format!("can't send {}: ", path.display())),
            "got: {message}"
        );
    }

    #[test]
    fn the_default_limit_allows_exactly_500_mb_and_refuses_anything_over() {
        assert_eq!(too_large(DEFAULT_MAX_FILE_SIZE, SizeLimit::Default), None);
        let reason = too_large(DEFAULT_MAX_FILE_SIZE + 1, SizeLimit::Default).unwrap();
        assert!(reason.starts_with("too large ("), "got: {reason}");
        assert!(reason.contains("max 500.0 MB"), "got: {reason}");
    }

    #[test]
    fn a_refusal_says_how_to_get_past_the_limit() {
        let reason = too_large(2048, SizeLimit::Bytes(1024)).unwrap();

        assert_eq!(
            reason,
            "too large (2.0 KB, max 1.0 KB) -- raise the limit with /maxsize <size> (say /maxsize 2GB) or lift it with /maxsize off"
        );
    }

    #[test]
    fn a_custom_limit_replaces_the_default_in_both_directions() {
        // Lowered below the default...
        assert!(too_large(2048, SizeLimit::Bytes(1024)).is_some());
        assert_eq!(
            too_large(1024, SizeLimit::Bytes(1024)),
            None,
            "a file of exactly the limit is allowed"
        );
        // ...and raised above it.
        let double = SizeLimit::Bytes(2 * DEFAULT_MAX_FILE_SIZE);
        assert_eq!(too_large(DEFAULT_MAX_FILE_SIZE + 1, double), None);
        assert!(too_large(2 * DEFAULT_MAX_FILE_SIZE + 1, double).is_some());
    }

    #[test]
    fn with_the_limit_off_nothing_is_too_large() {
        assert_eq!(too_large(u64::MAX, SizeLimit::Off), None);
    }

    #[test]
    fn max_bytes_is_the_default_a_chosen_count_or_nothing() {
        assert_eq!(SizeLimit::Default.max_bytes(), Some(DEFAULT_MAX_FILE_SIZE));
        assert_eq!(SizeLimit::Bytes(7).max_bytes(), Some(7));
        assert_eq!(SizeLimit::Off.max_bytes(), None);
    }

    #[test]
    fn stage_file_refuses_a_file_over_the_limit_and_stages_it_once_the_limit_allows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mkv");
        std::fs::write(&path, [0u8; 2048]).unwrap();

        let refused = stage_file(&path, SizeLimit::Bytes(1024)).unwrap_err();

        assert!(
            refused.starts_with(&format!(
                "can't send {}: too large (2.0 KB, max 1.0 KB)",
                path.display()
            )),
            "got: {refused}"
        );
        assert!(refused.contains("/maxsize"), "got: {refused}");
        for limit in [SizeLimit::Bytes(2048), SizeLimit::Off] {
            assert_eq!(stage_file(&path, limit).unwrap().size, 2048, "{limit:?}");
        }
    }

    #[test]
    fn stage_image_keeps_the_bytes_and_reports_their_size() {
        let staged = stage_image(
            "clipboard-1.png".to_string(),
            vec![1, 2, 3],
            SizeLimit::Default,
        )
        .unwrap();

        assert_eq!(
            staged,
            StagedAttachment {
                filename: "clipboard-1.png".to_string(),
                size: 3,
                source: StagedSource::Bytes(vec![1, 2, 3]),
            }
        );
    }

    #[test]
    fn stage_image_obeys_the_same_limit() {
        let name = || "clipboard-1.png".to_string();

        let refused = stage_image(name(), vec![0; 2048], SizeLimit::Bytes(1024)).unwrap_err();

        assert!(
            refused.starts_with("can't paste image: too large (2.0 KB, max 1.0 KB)"),
            "got: {refused}"
        );
        assert!(refused.contains("/maxsize"), "got: {refused}");
        assert!(stage_image(name(), vec![0; 2048], SizeLimit::Off).is_ok());
    }

    #[test]
    fn a_size_is_a_number_and_a_unit_in_any_case_with_or_without_a_blank() {
        for (typed, bytes) in [
            ("750MB", 750 << 20),
            ("750 MB", 750 << 20),
            ("750mb", 750 << 20),
            ("  2GB  ", 2 << 30),
            ("1.5GB", 3 << 29),
            ("500.5MB", 524_812_288),
            ("0.5 KB", 512),
            ("1TB", 1 << 40),
            ("1024B", 1024),
            ("1b", 1),
        ] {
            assert_eq!(
                SizeLimit::parse(typed),
                Ok(SizeLimit::Bytes(bytes)),
                "{typed:?}"
            );
        }
    }

    #[test]
    fn default_and_off_are_words_in_any_case() {
        assert_eq!(SizeLimit::parse("default"), Ok(SizeLimit::Default));
        assert_eq!(SizeLimit::parse("Default"), Ok(SizeLimit::Default));
        assert_eq!(SizeLimit::parse("off"), Ok(SizeLimit::Off));
        assert_eq!(SizeLimit::parse(" OFF "), Ok(SizeLimit::Off));
    }

    #[test]
    fn what_is_not_a_size_is_refused_with_what_to_type_instead() {
        for typed in [
            "", "banana", "5", "MB", "2XB", "2 G", "-5MB", "+5MB", "1,5GB", "1.2.3GB", "1e3MB",
            "2GB junk", ".MB", "inf", "NaN",
        ] {
            let message = SizeLimit::parse(typed).unwrap_err();

            assert!(
                message.contains(&format!("'{}'", typed.trim())) && message.contains("750MB"),
                "{typed:?} -> {message}"
            );
        }
    }

    #[test]
    fn a_limit_that_would_refuse_every_file_is_refused() {
        // The last is 1e-13 TB: about a tenth of a byte.
        for typed in ["0B", "0GB", "0.4B", "0.0000000000001TB"] {
            let message = SizeLimit::parse(typed).unwrap_err();

            assert!(
                message.contains("refuse every file"),
                "{typed:?} -> {message}"
            );
        }
    }

    #[test]
    fn a_size_too_big_to_count_is_refused_and_off_is_offered_instead() {
        let huge = format!("{}GB", "9".repeat(400));
        for typed in ["17000000TB", "99999999999999999999999999TB", huge.as_str()] {
            let message = SizeLimit::parse(typed).unwrap_err();

            assert!(
                message.contains("too big to count") && message.contains("/maxsize off"),
                "{message}"
            );
        }
        // The largest that still fits is taken.
        assert!(SizeLimit::parse("16000000TB").is_ok());
    }

    #[test]
    fn the_size_limit_is_the_default_until_one_is_set() {
        let data = tempfile::tempdir().unwrap();

        assert_eq!(size_limit(data.path()).unwrap(), SizeLimit::Default);
    }

    #[test]
    fn a_size_limit_that_was_set_is_what_a_restart_reads_back() {
        let data = tempfile::tempdir().unwrap();
        for limit in [
            SizeLimit::Bytes(1_048_576),
            SizeLimit::Bytes(3 << 29),
            SizeLimit::Off,
            SizeLimit::Default,
        ] {
            set_size_limit(data.path(), limit).unwrap();

            assert_eq!(size_limit(data.path()).unwrap(), limit, "{limit:?}");
        }
    }

    #[test]
    fn going_back_to_the_default_remembers_nothing() {
        let data = tempfile::tempdir().unwrap();
        set_size_limit(data.path(), SizeLimit::Off).unwrap();
        assert!(data.path().join(SIZE_LIMIT_FILE).exists());

        set_size_limit(data.path(), SizeLimit::Default).unwrap();
        assert!(!data.path().join(SIZE_LIMIT_FILE).exists());

        // And again, with nothing there to forget.
        set_size_limit(data.path(), SizeLimit::Default).unwrap();
    }

    #[test]
    fn a_size_limit_setting_that_is_not_a_limit_falls_back_to_the_guarded_default() {
        let data = tempfile::tempdir().unwrap();
        for content in ["", "\n", "banana", "0", "-5MB", "unlimited", "5"] {
            std::fs::write(data.path().join(SIZE_LIMIT_FILE), content).unwrap();

            assert_eq!(
                size_limit(data.path()).unwrap(),
                SizeLimit::Default,
                "{content:?}"
            );
        }
    }

    #[test]
    fn a_size_limit_setting_that_cannot_be_read_is_an_error_not_a_quiet_default() {
        let data = tempfile::tempdir().unwrap();
        // A folder where the file should be: not "missing", so not the default.
        std::fs::create_dir(data.path().join(SIZE_LIMIT_FILE)).unwrap();

        let err = size_limit(data.path()).unwrap_err();

        assert!(err.to_string().contains(SIZE_LIMIT_FILE), "got: {err}");
    }

    #[test]
    fn a_size_limit_that_cannot_be_remembered_says_so() {
        let data = tempfile::tempdir().unwrap();
        // A folder where the file should be: writing there fails.
        std::fs::create_dir(data.path().join(SIZE_LIMIT_FILE)).unwrap();

        let message = set_size_limit(data.path(), SizeLimit::Off).unwrap_err();

        assert!(
            message.starts_with("can't remember the size limit: "),
            "got: {message}"
        );
    }

    /// Writes a one-byte file called `name` into `dir` and returns its path.
    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"x").unwrap();
        path
    }

    /// `path` as Terminal.app and iTerm2 paste a dropped file: a backslash
    /// before each space and parenthesis.
    fn backslash_escaped(path: &Path) -> String {
        path.to_str()
            .unwrap()
            .replace(' ', "\\ ")
            .replace('(', "\\(")
            .replace(')', "\\)")
    }

    /// `path` as a `file://` URI, the way a drop from some file managers
    /// arrives.
    fn file_uri(path: &Path) -> String {
        let path = path.to_str().unwrap();
        format!(
            "file://{}",
            path.replace(' ', "%20")
                .replace('(', "%28")
                .replace(')', "%29")
        )
    }

    #[test]
    fn shell_words_splits_on_ascii_whitespace_only() {
        assert_eq!(
            shell_words("  a b\tc\nd \r\n").unwrap(),
            ["a", "b", "c", "d"]
        );
        // A Unicode space belongs to its word -- see `shell_words`.
        assert_eq!(shell_words("a\u{202f}b c").unwrap(), ["a\u{202f}b", "c"]);
        assert_eq!(shell_words(" \n ").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn shell_words_reads_backslash_escapes_and_quotes() {
        assert_eq!(
            shell_words(r"/a/My\ File\ \(1\).png").unwrap(),
            ["/a/My File (1).png"]
        );
        assert_eq!(
            shell_words(r#"'/a/b c.txt' "/a/d e.txt""#).unwrap(),
            ["/a/b c.txt", "/a/d e.txt"]
        );
        // Forms mix inside one word, as a shell reads them.
        assert_eq!(shell_words(r#"/a/'b c'/d\ e"#).unwrap(), ["/a/b c/d e"]);
    }

    #[test]
    fn shell_words_double_quotes_only_escape_four_characters() {
        assert_eq!(
            shell_words(r#""hello (\$world)""#).unwrap(),
            ["hello ($world)"]
        );
        assert_eq!(shell_words(r#""say \"hi\"""#).unwrap(), [r#"say "hi""#]);
        // A backslash before anything else is kept, which leaves a Windows
        // path intact.
        assert_eq!(shell_words(r#""C:\a b\c.txt""#).unwrap(), [r"C:\a b\c.txt"]);
    }

    #[test]
    fn shell_words_keeps_an_explicitly_empty_word() {
        assert_eq!(shell_words("'' a").unwrap(), ["", "a"]);
    }

    #[test]
    fn shell_words_refuses_what_a_shell_would_still_be_waiting_on() {
        assert_eq!(shell_words("'open"), None);
        assert_eq!(shell_words("\"open"), None);
        assert_eq!(shell_words("trailing\\"), None);
    }

    #[test]
    fn word_to_path_takes_an_absolute_path_and_refuses_anything_else() {
        assert_eq!(
            word_to_path("/tmp/a.txt"),
            Some(PathBuf::from("/tmp/a.txt"))
        );
        assert_eq!(word_to_path("a.txt"), None);
        assert_eq!(word_to_path("./a.txt"), None);
        assert_eq!(word_to_path("~/a.txt"), None);
        assert_eq!(word_to_path("https://example.com/a.png"), None);
        assert_eq!(word_to_path(""), None);
    }

    #[test]
    fn word_to_path_decodes_a_file_uri_for_this_machine() {
        assert_eq!(
            word_to_path("file:///tmp/a%20b.txt"),
            Some(PathBuf::from("/tmp/a b.txt"))
        );
        assert_eq!(
            word_to_path("file://localhost/tmp/x.txt"),
            Some(PathBuf::from("/tmp/x.txt"))
        );
        // Percent-encoded UTF-8: an e with an acute accent.
        assert_eq!(
            word_to_path("file:///tmp/caf%C3%A9.txt"),
            Some(PathBuf::from("/tmp/caf\u{e9}.txt"))
        );
    }

    #[test]
    fn word_to_path_refuses_a_remote_or_malformed_uri() {
        assert_eq!(word_to_path("file://example.com/tmp/x.txt"), None);
        assert_eq!(word_to_path("file:///tmp/%zz.txt"), None);
        assert_eq!(word_to_path("file:///tmp/100%"), None);
        // Decodes to a byte that is not valid UTF-8.
        assert_eq!(word_to_path("file:///tmp/%ff.txt"), None);
    }

    #[test]
    fn dropped_files_reads_a_single_file_in_every_terminal_spelling() {
        let dir = tempfile::tempdir().unwrap();
        let path = touch(dir.path(), "My File (1).txt");
        let raw = path.to_str().unwrap();
        let spellings = [
            // Terminal.app, iTerm2, Ghostty: backslash-escaped, then a space.
            format!("{} ", backslash_escaped(&path)),
            // iTerm2's quoting option, GNOME Terminal: single quotes.
            format!("'{raw}'"),
            // WezTerm's Posix mode, Windows Terminal: double quotes.
            format!("\"{raw}\""),
            // Some file managers: a file:// URI.
            file_uri(&path),
            // kitty, older Alacritty: the path as it is, spaces and all.
            raw.to_string(),
            format!("{raw} "),
        ];

        for spelling in spellings {
            assert_eq!(
                dropped_files(&spelling),
                Some(vec![path.clone()]),
                "spelling: {spelling:?}"
            );
        }
    }

    #[test]
    fn dropped_files_reads_several_files_joined_by_spaces_or_newlines() {
        let dir = tempfile::tempdir().unwrap();
        let first = touch(dir.path(), "a.txt");
        let second = touch(dir.path(), "b b.txt");
        let both = Some(vec![first.clone(), second.clone()]);

        // Escaped and space-joined, as Terminal.app and iTerm2 paste them.
        let spaced = format!(
            "{} {} ",
            backslash_escaped(&first),
            backslash_escaped(&second)
        );
        assert_eq!(dropped_files(&spaced), both);
        // Quoted and newline-joined.
        let quoted = format!("'{}'\n'{}'\n", first.display(), second.display());
        assert_eq!(dropped_files(&quoted), both);
        // One raw path per line, spaces unescaped.
        let raw = format!("{}\n{}", first.display(), second.display());
        assert_eq!(dropped_files(&raw), both);
        // A CRLF-joined list of URIs.
        let uris = format!("{}\r\n{}\r\n", file_uri(&first), file_uri(&second));
        assert_eq!(dropped_files(&uris), both);
    }

    #[test]
    fn dropped_files_keeps_a_name_with_a_narrow_no_break_space_whole() {
        // macOS puts U+202F before the AM/PM in a screenshot's name, and a
        // terminal does not escape it -- so it must not split the path.
        let dir = tempfile::tempdir().unwrap();
        let path = touch(dir.path(), "Screenshot 2026-10-12 at 8.01.02\u{202f}AM.png");
        let escaped_spaces = path.to_str().unwrap().replace(' ', "\\ ");

        assert_eq!(dropped_files(&escaped_spaces), Some(vec![path.clone()]));
        assert_eq!(
            dropped_files(&format!("{} ", path.display())),
            Some(vec![path])
        );
    }

    #[test]
    fn dropped_files_is_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let real = touch(dir.path(), "real.txt");
        let missing = dir.path().join("missing.txt");

        // One path that is not there makes the whole paste text...
        let with_missing = format!("{} {}", real.display(), missing.display());
        assert_eq!(dropped_files(&with_missing), None);
        // ... so does a word that is not a path at all...
        assert_eq!(dropped_files(&format!("{} please", real.display())), None);
        // ... and so does a sentence that merely mentions a real file.
        let sentence = format!("see {} for the details", real.display());
        assert_eq!(dropped_files(&sentence), None);
    }

    #[test]
    fn dropped_files_ignores_directories_relative_names_urls_and_blank_text() {
        let dir = tempfile::tempdir().unwrap();

        // A directory is not a file to send; pasting its path is just text.
        assert_eq!(dropped_files(dir.path().to_str().unwrap()), None);
        // A relative name is only a word, even though this one matches a
        // file in the working directory (cargo test runs from the package
        // root).
        assert_eq!(dropped_files("Cargo.toml"), None);
        assert_eq!(dropped_files("https://example.com/a.png"), None);
        assert_eq!(dropped_files(""), None);
        assert_eq!(dropped_files(" \n\t "), None);
    }
}
