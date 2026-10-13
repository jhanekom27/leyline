//! Local channel-folder inventory for sync publication.
//!
//! This module only observes local regular files. The runtime runs scans on
//! Tokio's blocking pool, and networking owns blob imports and index writes.
//! Deletes and renames deliberately produce no publication action.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub channel: String,
    pub path: PathBuf,
    pub relative: String,
    pub size: u64,
    pub modified: SystemTime,
}

/// Recursively inventories eligible regular files below a channel folder.
/// Hidden, editor-temporary, and outbox files are deliberately local-only.
pub fn scan(channel: String, root: PathBuf) -> Result<Vec<Candidate>, String> {
    let mut candidates = Vec::new();
    if !root.exists() {
        return Ok(candidates);
    }
    scan_directory(&channel, &root, &root, &mut candidates)?;
    Ok(candidates)
}

fn scan_directory(
    channel: &str,
    root: &Path,
    directory: &Path,
    candidates: &mut Vec<Candidate>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|err| format!("can't read sync folder {}: {err}", directory.display()))?
    {
        let entry = entry
            .map_err(|err| format!("can't read sync folder {}: {err}", directory.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if ignored_name(&name) {
            continue;
        }
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| format!("can't inspect sync path {}: {err}", path.display()))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            // The outbox is a separate local-only convention.
            if name != "send" {
                scan_directory(channel, root, &path, candidates)?;
            }
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .expect("recursive path must remain below root")
            .to_str()
            .ok_or_else(|| format!("sync skipped non-UTF-8 path {}", path.display()))?
            .replace(std::path::MAIN_SEPARATOR, "/");
        candidates.push(Candidate {
            channel: channel.to_string(),
            path,
            relative,
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    Ok(())
}

/// Releases only candidates unchanged since the immediately preceding scan.
#[derive(Default)]
pub struct Watcher {
    previous: HashMap<(String, String), (u64, SystemTime)>,
    remote: HashMap<(String, String), Option<(u64, SystemTime)>>,
}

impl Watcher {
    /// Records a destination written by remote sync. Its first subsequent
    /// inventory supplies the local timestamp; it remains suppressed until a
    /// later scan observes a changed size or timestamp.
    pub fn suppress_remote(&mut self, channel: String, relative: String) {
        self.remote.insert((channel, relative), None);
    }
    pub fn stable(&mut self, candidates: Vec<Candidate>) -> Vec<Candidate> {
        let mut current = HashMap::new();
        let mut stable = Vec::new();
        for candidate in candidates {
            let key = (candidate.channel.clone(), candidate.relative.clone());
            let stamp = (candidate.size, candidate.modified);
            let changed_remote = if let Some(remote_stamp) = self.remote.get_mut(&key) {
                match remote_stamp {
                    None => {
                        *remote_stamp = Some(stamp);
                        current.insert(key, stamp);
                        continue;
                    }
                    Some(known) if known == &stamp => {
                        current.insert(key, stamp);
                        continue;
                    }
                    Some(_) => true,
                }
            } else {
                false
            };
            if changed_remote {
                self.remote.remove(&key);
            }
            if self.previous.get(&key) == Some(&stamp) {
                stable.push(candidate.clone());
            }
            current.insert(key, stamp);
        }
        self.previous = current;
        stable
    }
}

fn ignored_name(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with("~$")
        || name.ends_with('~')
        || name.ends_with(".part")
        || name.ends_with(".crdownload")
        || name.ends_with(".partial")
        || name.ends_with(".tmp")
        || name.ends_with(".swp")
        || name.ends_with(".swo")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_is_recursive_and_skips_local_only_and_temporary_paths() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("nested")).unwrap();
        fs::create_dir_all(root.path().join("send")).unwrap();
        fs::create_dir_all(root.path().join(".hidden")).unwrap();
        fs::write(root.path().join("nested/report.txt"), b"report").unwrap();
        fs::write(root.path().join("send/outbox.txt"), b"outbox").unwrap();
        fs::write(root.path().join(".hidden/secret.txt"), b"hidden").unwrap();
        fs::write(root.path().join("draft.swp"), b"temp").unwrap();

        let found = scan("general".into(), root.path().to_path_buf()).unwrap();

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].relative, "nested/report.txt");
    }

    #[test]
    fn watcher_requires_two_unchanged_scans_and_releases_edits_again() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("note.txt");
        fs::write(&path, b"first").unwrap();
        let mut watcher = Watcher::default();

        assert!(
            watcher
                .stable(scan("general".into(), root.path().to_path_buf()).unwrap())
                .is_empty()
        );
        assert_eq!(
            watcher
                .stable(scan("general".into(), root.path().to_path_buf()).unwrap())
                .len(),
            1
        );

        fs::write(&path, b"a changed file").unwrap();
        assert!(
            watcher
                .stable(scan("general".into(), root.path().to_path_buf()).unwrap())
                .is_empty()
        );
        assert_eq!(
            watcher
                .stable(scan("general".into(), root.path().to_path_buf()).unwrap())
                .len(),
            1
        );
    }

    #[test]
    fn absent_folder_is_empty_until_a_restart_or_enable_scan_finds_files() {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("general");

        assert!(scan("general".into(), folder.clone()).unwrap().is_empty());

        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("after-restart.txt"), b"picked up").unwrap();
        let mut watcher = Watcher::default();
        assert!(
            watcher
                .stable(scan("general".into(), folder.clone()).unwrap())
                .is_empty()
        );
        assert_eq!(
            watcher.stable(scan("general".into(), folder).unwrap())[0].relative,
            "after-restart.txt"
        );
    }

    #[test]
    fn remote_destination_stays_suppressed_until_edited() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("remote conflict.txt");
        fs::write(&path, b"remote").unwrap();
        let mut watcher = Watcher::default();
        watcher.suppress_remote("general".into(), "remote conflict.txt".into());

        assert!(
            watcher
                .stable(scan("general".into(), root.path().to_path_buf()).unwrap())
                .is_empty()
        );
        assert!(
            watcher
                .stable(scan("general".into(), root.path().to_path_buf()).unwrap())
                .is_empty()
        );

        fs::write(&path, b"local edit").unwrap();
        assert!(
            watcher
                .stable(scan("general".into(), root.path().to_path_buf()).unwrap())
                .is_empty()
        );
        assert_eq!(
            watcher
                .stable(scan("general".into(), root.path().to_path_buf()).unwrap())
                .len(),
            1
        );
    }
}
