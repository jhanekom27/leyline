//! Local filesystem policy for shared files -- see features.md's "File
//! sharing" idea. Kept separate from backfill.rs (which only knows about
//! content-addressed blobs, not real paths) and net.rs (transport): this is
//! where a peer's announced filename turns into an actual, safe path on
//! disk, and where "where did it go" gets answered with zero configuration.

use std::fs::OpenOptions;
use std::path::{Path, PathBuf};

/// Subfolder created under the OS Downloads directory (or the fallback
/// below) so everything leyline has ever saved lives in one place instead
/// of cluttering the top-level Downloads folder.
const SUBDIR: &str = "leyline";

/// How many "name (n).ext" candidates to try before giving up -- a safety
/// valve against a pathological loop, not a realistic limit.
const MAX_COLLISION_ATTEMPTS: u32 = 1000;

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
}
