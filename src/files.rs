//! Local filesystem policy for shared files -- see features.md's "File
//! sharing" idea. Kept separate from backfill.rs (which only knows about
//! content-addressed blobs, not real paths) and net.rs (transport): this is
//! where a peer's announced filename turns into an actual, safe path on
//! disk, and where "where did it go" gets answered with zero configuration.

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
    std::fs::create_dir_all(dir)?;

    let safe_name = sanitize_filename(filename);
    let candidate = dir.join(&safe_name);
    if !candidate.exists() {
        return Ok(candidate);
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
        let candidate = dir.join(candidate_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(std::io::Error::other(format!(
        "could not find a free name for {safe_name:?} in {}",
        dir.display()
    )))
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
    fn human_size_formats_bytes_and_larger_units() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KB");
        assert_eq!(human_size(2_150_000), "2.1 MB");
    }
}
