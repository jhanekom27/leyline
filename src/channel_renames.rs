//! Persisted local display names for group channels: a mapping from a
//! channel's stored name to a name you've given it yourself with
//! `/rename`, never sent over the wire -- the channel counterpart to
//! `contacts.rs`'s pet names for peers, and applied the same way, by
//! `AppState::channel_label`.
//!
//! Mirrors `contacts.rs`'s persisted-postcard-file pattern: small,
//! infrequently-changed data, so it's simplest to just re-encode and
//! rewrite the whole file on every change.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// One assigned display name: the stored name of the channel it renames
/// (its identity -- see `AppState::channel_label`), and the name itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RenameRecord {
    channel: String,
    name: String,
}

/// Local, whole-file-rewrite-on-change store of channel display names.
///
/// Expected to stay small (a handful of renamed channels), so like
/// `contacts.rs`, it's simplest to just re-encode and rewrite the whole
/// file on every change.
pub struct ChannelRenames {
    path: PathBuf,
    renames: Vec<RenameRecord>,
}

impl ChannelRenames {
    /// Loads the renames persisted at `path`, or starts empty if the file
    /// doesn't exist yet.
    ///
    /// A corrupt file is logged and treated as empty rather than failing
    /// startup -- like `contacts.rs`, losing it just means channels go back
    /// to showing their stored names until renamed again, not any real
    /// data loss. Only a genuine I/O error (e.g. permission denied) is
    /// returned as `Err`.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let renames = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, renames })
    }

    /// All display names, keyed by the stored name of the channel they
    /// rename -- seeded into `AppState` once at startup (see
    /// `AppState::load_renames`).
    pub fn all(&self) -> HashMap<String, String> {
        self.renames
            .iter()
            .map(|r| (r.channel.clone(), r.name.clone()))
            .collect()
    }

    /// Gives `channel` the display name `name`, overwriting any already
    /// recorded for it (so re-running `/rename` renames again).
    pub fn set(&mut self, channel: String, name: String) -> anyhow::Result<()> {
        match self.renames.iter_mut().find(|r| r.channel == channel) {
            Some(existing) => existing.name = name,
            None => self.renames.push(RenameRecord { channel, name }),
        }
        self.save()
    }

    /// Forgets `channel`'s display name -- called both when a bare
    /// `/rename` restores the stored name and when the channel is left, so
    /// a later channel under the same stored name never inherits a stale
    /// one. A no-op (including no write to disk) if `channel` has none.
    pub fn forget(&mut self, channel: &str) -> anyhow::Result<()> {
        let before = self.renames.len();
        self.renames.retain(|r| r.channel != channel);
        if self.renames.len() == before {
            return Ok(());
        }
        self.save()
    }

    /// Re-encodes and rewrites the whole renames file.
    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes =
            postcard::to_stdvec(&self.renames).context("failed to encode channel renames")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write channel renames to {}", self.path.display()))
    }
}

/// Decodes the renames file's contents, tolerating corruption by logging
/// and falling back to an empty list -- see `ChannelRenames::load`.
fn decode(bytes: &[u8]) -> Vec<RenameRecord> {
    postcard::from_bytes(bytes).unwrap_or_else(|err| {
        warn!("dropping corrupt channel renames file: {err}");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renames_at(dir: &tempfile::TempDir) -> ChannelRenames {
        ChannelRenames::load(dir.path().join("channel_renames")).unwrap()
    }

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(channel, name)| (channel.to_string(), name.to_string()))
            .collect()
    }

    #[test]
    fn missing_file_is_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        assert!(renames_at(&dir).all().is_empty());
    }

    #[test]
    fn corrupt_file_is_tolerated_as_an_empty_list() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("channel_renames"),
            b"not a valid renames file",
        )
        .unwrap();
        assert!(renames_at(&dir).all().is_empty());
    }

    #[test]
    fn set_then_all_contains_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut renames = renames_at(&dir);
        renames
            .set("project-x".to_string(), "proj".to_string())
            .unwrap();
        assert_eq!(renames.all(), map(&[("project-x", "proj")]));
    }

    #[test]
    fn set_again_overwrites_the_existing_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut renames = renames_at(&dir);
        renames
            .set("project-x".to_string(), "proj".to_string())
            .unwrap();
        renames
            .set("project-x".to_string(), "px".to_string())
            .unwrap();
        assert_eq!(
            renames.all(),
            map(&[("project-x", "px")]),
            "renaming the same channel again must replace, not duplicate"
        );
    }

    #[test]
    fn channels_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let mut renames = renames_at(&dir);
        renames
            .set("project-x".to_string(), "proj".to_string())
            .unwrap();
        renames
            .set("general".to_string(), "home".to_string())
            .unwrap();
        assert_eq!(
            renames.all(),
            map(&[("project-x", "proj"), ("general", "home")])
        );
    }

    #[test]
    fn forget_removes_only_that_channels_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut renames = renames_at(&dir);
        renames
            .set("project-x".to_string(), "proj".to_string())
            .unwrap();
        renames
            .set("general".to_string(), "home".to_string())
            .unwrap();

        renames.forget("project-x").unwrap();

        assert_eq!(renames.all(), map(&[("general", "home")]));
    }

    #[test]
    fn forget_is_a_no_op_for_a_channel_with_no_name_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut renames = renames_at(&dir);

        renames.forget("project-x").unwrap();

        assert!(renames.all().is_empty());
        assert!(
            !dir.path().join("channel_renames").exists(),
            "forgetting nothing must not create the file"
        );
    }

    #[test]
    fn persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel_renames");

        let mut renames = ChannelRenames::load(path.clone()).unwrap();
        renames
            .set("project-x".to_string(), "proj".to_string())
            .unwrap();

        let reloaded = ChannelRenames::load(path).unwrap();
        assert_eq!(reloaded.all(), map(&[("project-x", "proj")]));
    }

    #[test]
    fn forget_persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("channel_renames");

        let mut renames = ChannelRenames::load(path.clone()).unwrap();
        renames
            .set("project-x".to_string(), "proj".to_string())
            .unwrap();
        renames.forget("project-x").unwrap();

        let reloaded = ChannelRenames::load(path).unwrap();
        assert!(reloaded.all().is_empty());
    }
}
