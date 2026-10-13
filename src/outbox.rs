//! Persisted one-way outbox settings and polling helpers.
//!
//! An outbox is just an enabled channel plus the names that have already
//! been observed there. The watcher is deliberately one-way: it never moves,
//! deletes, or copies files back into the folder.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChannelOutbox {
    channel: String,
    seen: Vec<String>,
}

/// Local, whole-file-rewrite persistence for enabled channel outboxes.
pub struct Outbox {
    path: PathBuf,
    channels: Vec<ChannelOutbox>,
}

impl Outbox {
    /// Loads persisted outboxes. A corrupt setting is safe to discard: it
    /// leaves every outbox off instead of watching a folder unexpectedly.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let channels = match fs::read(&path) {
            Ok(bytes) => postcard::from_bytes(&bytes).unwrap_or_else(|err| {
                warn!("dropping corrupt outbox settings: {err}");
                Vec::new()
            }),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, channels })
    }

    pub fn channels(&self) -> HashSet<String> {
        self.channels
            .iter()
            .map(|entry| entry.channel.clone())
            .collect()
    }

    /// Creates `folder` and enables its channel. Files already present are
    /// recorded before the setting is saved, so turning an outbox on is never
    /// retroactive.
    pub fn enable(&mut self, channel: &str, folder: &Path) -> anyhow::Result<()> {
        fs::create_dir_all(folder)
            .with_context(|| format!("failed to create {}", folder.display()))?;
        let seen = initial_names(folder)?;
        self.channels.retain(|entry| entry.channel != channel);
        self.channels.push(ChannelOutbox {
            channel: channel.to_string(),
            seen,
        });
        self.save()
    }

    /// Disables an outbox and forgets its record. Re-enabling snapshots what
    /// is then in the folder, exactly like a first enable.
    pub fn disable(&mut self, channel: &str) -> anyhow::Result<()> {
        let before = self.channels.len();
        self.channels.retain(|entry| entry.channel != channel);
        if self.channels.len() != before {
            self.save()?;
        }
        Ok(())
    }

    pub fn seen(&self, channel: &str, name: &str) -> bool {
        self.channels
            .iter()
            .find(|entry| entry.channel == channel)
            .is_some_and(|entry| entry.seen.iter().any(|known| known == name))
    }

    /// Records an eligible file before it is handed to the sender. That
    /// makes a restart or a later edit unable to create a duplicate share.
    pub fn mark_seen(&mut self, channel: &str, name: String) -> anyhow::Result<()> {
        let Some(entry) = self
            .channels
            .iter_mut()
            .find(|entry| entry.channel == channel)
        else {
            return Ok(());
        };
        if entry.seen.iter().any(|known| known == &name) {
            return Ok(());
        }
        entry.seen.push(name);
        self.save()
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes =
            postcard::to_stdvec(&self.channels).context("failed to encode outbox settings")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }
}

/// A regular, eligible file found by the background directory scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub channel: String,
    pub path: PathBuf,
    pub name: String,
    size: u64,
    modified: SystemTime,
}

/// Reads one outbox folder. Directory access happens on the blocking pool;
/// content validation stays in the existing `files::stage_file` boundary.
pub fn scan(channel: String, folder: PathBuf) -> Result<Vec<Candidate>, String> {
    let entries = fs::read_dir(&folder)
        .map_err(|err| format!("can't read outbox {}: {err}", folder.display()))?;
    let mut candidates = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|err| format!("can't read outbox {}: {err}", folder.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if ignored_name(&name) {
            continue;
        }
        let path = entry.path();
        let metadata = match entry.metadata() {
            Ok(metadata) if metadata.is_file() => metadata,
            Ok(_) => continue,
            Err(err) => {
                return Err(format!(
                    "can't inspect outbox file {}: {err}",
                    path.display()
                ));
            }
        };
        candidates.push(Candidate {
            channel: channel.clone(),
            path,
            name,
            size: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        });
    }
    Ok(candidates)
}

/// Remembers one scan's metadata. A candidate must match a later scan before
/// it is released, so a file still being copied is never sent half-written.
#[derive(Default)]
pub struct Watcher {
    previous: HashMap<(String, String), (u64, SystemTime)>,
}

impl Watcher {
    pub fn stable(&mut self, candidates: Vec<Candidate>, outbox: &Outbox) -> Vec<Candidate> {
        let mut current = HashMap::new();
        let mut stable = Vec::new();
        for candidate in candidates {
            let key = (candidate.channel.clone(), candidate.name.clone());
            let stamp = (candidate.size, candidate.modified);
            if !outbox.seen(&candidate.channel, &candidate.name)
                && self.previous.get(&key) == Some(&stamp)
            {
                stable.push(candidate.clone());
            }
            current.insert(key, stamp);
        }
        self.previous = current;
        stable
    }
}

fn initial_names(folder: &Path) -> anyhow::Result<Vec<String>> {
    let entries =
        fs::read_dir(folder).with_context(|| format!("failed to read {}", folder.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("failed to read {}", folder.display()))?;
        if entry.metadata().is_ok_and(|metadata| metadata.is_file()) {
            names.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    Ok(names)
}

fn ignored_name(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with("~$")
        || name.ends_with(".part")
        || name.ends_with(".crdownload")
        || name.ends_with(".partial")
        || name.ends_with(".tmp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enabling_snapshots_existing_files_and_persists_the_setting() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("send");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("already.txt"), b"old").unwrap();
        let path = dir.path().join("outbox");
        let mut outbox = Outbox::load(path.clone()).unwrap();

        outbox.enable("general", &folder).unwrap();

        assert!(outbox.channels().contains("general"));
        assert!(outbox.seen("general", "already.txt"));
        assert!(Outbox::load(path).unwrap().seen("general", "already.txt"));
    }

    #[test]
    fn watcher_requires_two_matching_scans_and_ignores_temporary_names() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("send");
        fs::create_dir_all(&folder).unwrap();
        let mut outbox = Outbox::load(dir.path().join("outbox")).unwrap();
        outbox.enable("general", &folder).unwrap();
        fs::write(folder.join("new.txt"), b"new").unwrap();
        fs::write(folder.join(".DS_Store"), b"ignored").unwrap();
        fs::write(folder.join("copy.part"), b"ignored").unwrap();
        let mut watcher = Watcher::default();

        assert!(
            watcher
                .stable(scan("general".into(), folder.clone()).unwrap(), &outbox)
                .is_empty()
        );
        let stable = watcher.stable(scan("general".into(), folder).unwrap(), &outbox);

        assert_eq!(stable.len(), 1);
        assert_eq!(stable[0].name, "new.txt");
    }
}
