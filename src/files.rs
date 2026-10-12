//! Local filesystem policy for shared files -- see features.md's "File
//! sharing" idea. Kept separate from backfill.rs (which only knows about
//! content-addressed blobs, not real paths) and net.rs (transport): this is
//! where a peer's announced filename turns into an actual, safe path on
//! disk, where "where did it go" gets answered with zero configuration,
//! and -- on the way out -- where a file someone wants to share is checked
//! before it is staged (`stage_file`/`stage_image`).

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

/// Subfolder created under the OS Downloads directory (or the fallback
/// below) so everything leyline has ever saved lives in one place instead
/// of cluttering the top-level Downloads folder.
const SUBDIR: &str = "leyline";

/// How many "name (n).ext" candidates to try before giving up -- a safety
/// valve against a pathological loop, not a realistic limit.
const MAX_COLLISION_ATTEMPTS: u32 = 1000;

/// Largest file (or pasted image) that can be shared: a simple guardrail
/// against an accidental huge share (e.g. a mistyped path to a large video
/// file). `BackfillStore::add_file` copies the file into the local blob
/// store, so a shared file's bytes are duplicated on disk locally.
pub const MAX_FILE_SIZE: u64 = 500 * 1024 * 1024;

/// Where downloaded files land by default: the OS Downloads folder (via
/// `directories::UserDirs`) under a `leyline/` subfolder, so a saved file
/// ends up somewhere the user would actually think to look, with zero
/// configuration. Falls back to `fallback_base` (leyline's own data
/// directory) if no Downloads folder can be resolved at all -- e.g. no
/// resolvable home directory -- so saving always has somewhere to go.
pub fn downloads_dir(fallback_base: &Path) -> PathBuf {
    directories::UserDirs::new()
        .and_then(|dirs| dirs.download_dir().map(|dir| dir.join(SUBDIR)))
        .unwrap_or_else(|| fallback_base.join("downloads"))
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

/// Resolves a collision-free destination for `filename` inside `dir`,
/// creating `dir` if it doesn't exist yet. Never overwrites an existing
/// file and never prompts: if `filename` is already taken, tries
/// `name (1).ext`, `name (2).ext`, etc. -- the browser convention -- until
/// a free path is found.
pub fn resolve_destination(dir: &Path, filename: &str) -> std::io::Result<PathBuf> {
    if let Err(err) = std::fs::create_dir_all(dir) {
        // The one case worth a clearer message than the raw OS error:
        // `dir` (or one of its ancestors) is already a plain file rather
        // than a folder -- e.g. a `leyline` binary sitting directly in
        // Downloads, colliding with the `leyline/` subfolder `downloads_dir`
        // always asks for -- so `create_dir_all` can never succeed there
        // no matter how many times it's retried. Confirmed in the wild:
        // this is exactly what "File exists (os error 17)" meant here.
        if dir.is_file() {
            return Err(std::io::Error::other(format!(
                "{} already exists and isn't a folder -- rename it, remove it, or move it elsewhere so leyline can create its downloads folder there",
                dir.display()
            )));
        }
        return Err(err);
    }

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

/// Checks `path` (already expanded, see `expand_path`) for sharing and
/// stages it: it must exist, be a file rather than a directory, be
/// readable, fit within `MAX_FILE_SIZE`, and have a name to share it
/// under. Refusals read like the ones `/send` has always given, since
/// `/send` and the clipboard both stage through here -- one boundary, so
/// what each of them refuses and how it says so cannot drift apart.
pub fn stage_file(path: &Path) -> Result<StagedAttachment, String> {
    let metadata =
        std::fs::metadata(path).map_err(|err| format!("can't send {}: {err}", path.display()))?;
    if !metadata.is_file() {
        return Err(format!("can't send {}: not a file", path.display()));
    }
    // `metadata` works on a file whose contents can't be read (no read
    // permission), so try opening it: refusing now beats failing at import
    // time, after a caption has been typed.
    std::fs::File::open(path).map_err(|err| format!("can't send {}: {err}", path.display()))?;
    if let Some(reason) = too_large(metadata.len()) {
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
/// `net::Net::paste`) under `filename`, subject to the same
/// `MAX_FILE_SIZE` cap as `stage_file` -- there is no path to inspect, so
/// the size is the only thing to check.
pub fn stage_image(filename: String, bytes: Vec<u8>) -> Result<StagedAttachment, String> {
    let size = bytes.len() as u64;
    if let Some(reason) = too_large(size) {
        return Err(format!("can't paste image: {reason}"));
    }
    Ok(StagedAttachment {
        filename,
        size,
        source: StagedSource::Bytes(bytes),
    })
}

/// Why `size` bytes can't be shared, or `None` if they can -- the wording
/// shared by `stage_file`'s and `stage_image`'s refusals.
fn too_large(size: u64) -> Option<String> {
    (size > MAX_FILE_SIZE).then(|| {
        format!(
            "too large ({}, max {})",
            human_size(size),
            human_size(MAX_FILE_SIZE)
        )
    })
}

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

        let staged = stage_file(&path).unwrap();

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

        let message = stage_file(&missing).unwrap_err();

        assert!(
            message.starts_with(&format!("can't send {}: ", missing.display())),
            "got: {message}"
        );
    }

    #[test]
    fn stage_file_refuses_a_directory() {
        let dir = tempfile::tempdir().unwrap();

        let message = stage_file(dir.path()).unwrap_err();

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

        let message = stage_file(&path).unwrap_err();

        assert!(
            message.starts_with(&format!("can't send {}: ", path.display())),
            "got: {message}"
        );
    }

    #[test]
    fn too_large_only_flags_sizes_over_the_limit() {
        assert_eq!(too_large(MAX_FILE_SIZE), None);
        let reason = too_large(MAX_FILE_SIZE + 1).unwrap();
        assert!(reason.starts_with("too large ("), "got: {reason}");
        assert!(reason.contains("max 500.0 MB"), "got: {reason}");
    }

    #[test]
    fn stage_image_keeps_the_bytes_and_reports_their_size() {
        let staged = stage_image("clipboard-1.png".to_string(), vec![1, 2, 3]).unwrap();

        assert_eq!(
            staged,
            StagedAttachment {
                filename: "clipboard-1.png".to_string(),
                size: 3,
                source: StagedSource::Bytes(vec![1, 2, 3]),
            }
        );
    }
}
