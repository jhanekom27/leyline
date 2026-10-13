//! Persisted record of which channels have auto-save on (`/autosave`): the
//! stored names of those channels and nothing else -- off is simply not being
//! in the list. Local only, never sent over the wire.
//!
//! Keyed by a channel's stored name, as its folder is
//! (`files::channel_dir`), so it is forgotten when the channel is left
//! (`AutoSave::set(.., false)`): a ticket's name is chosen by whoever shared
//! it, and a setting that outlived its channel would switch downloads on for
//! a different room that happens to reuse the name, which nobody asked for.
//! Pairing wipes the file for the same reason (main.rs's
//! `paths_to_wipe_for_pairing`).
//!
//! Mirrors `channel_renames.rs`'s persisted-postcard-file pattern: tiny and
//! rarely changed, so the whole file is simply rewritten on every change.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use tracing::warn;

/// Local, whole-file-rewrite-on-change store of the channels with auto-save
/// on.
pub struct AutoSave {
    path: PathBuf,
    /// Stored channel names, each once.
    channels: Vec<String>,
}

impl AutoSave {
    /// Loads the channels persisted at `path`, or starts with none if the
    /// file doesn't exist yet.
    ///
    /// A corrupt file is logged and treated as empty rather than failing
    /// startup -- which is the safe way for it to fail: losing it only means
    /// auto-save is off everywhere until it is turned on again, never on
    /// where nobody asked. Only a genuine I/O error (e.g. permission denied)
    /// is returned as `Err`.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let channels = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, channels })
    }

    /// The stored names of the channels with auto-save on -- seeded into
    /// `AppState` once at startup (see `AppState::load_autosave`).
    pub fn channels(&self) -> HashSet<String> {
        self.channels.iter().cloned().collect()
    }

    /// Turns auto-save on or off for `channel`. A no-op (including no write
    /// to disk) if it is already as asked, so turning it off for a channel
    /// that never had it -- as leaving every channel does -- costs nothing.
    pub fn set(&mut self, channel: &str, on: bool) -> anyhow::Result<()> {
        if self.channels.iter().any(|known| known == channel) == on {
            return Ok(());
        }
        if on {
            self.channels.push(channel.to_string());
        } else {
            self.channels.retain(|known| known != channel);
        }
        self.save()
    }

    /// Re-encodes and rewrites the whole file.
    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes = postcard::to_stdvec(&self.channels).context("failed to encode auto-save")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write auto-save to {}", self.path.display()))
    }
}

/// Decodes the file's contents, tolerating corruption by logging and falling
/// back to an empty list -- see `AutoSave::load`.
fn decode(bytes: &[u8]) -> Vec<String> {
    postcard::from_bytes(bytes).unwrap_or_else(|err| {
        warn!("dropping corrupt auto-save file: {err}");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn autosave_at(dir: &tempfile::TempDir) -> AutoSave {
        AutoSave::load(dir.path().join("autosave")).unwrap()
    }

    fn set_of(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn missing_file_means_off_everywhere() {
        let dir = tempfile::tempdir().unwrap();
        assert!(autosave_at(&dir).channels().is_empty());
    }

    #[test]
    fn corrupt_file_is_tolerated_as_off_everywhere() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("autosave"), b"not a valid auto-save file").unwrap();
        assert!(autosave_at(&dir).channels().is_empty());
    }

    #[test]
    fn a_file_that_cannot_be_read_is_an_error_not_a_quiet_reset() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file should be: not "missing", so not defaulted.
        fs::create_dir(dir.path().join("autosave")).unwrap();

        assert!(AutoSave::load(dir.path().join("autosave")).is_err());
    }

    #[test]
    fn turning_it_on_adds_the_channel() {
        let dir = tempfile::tempdir().unwrap();
        let mut autosave = autosave_at(&dir);

        autosave.set("general", true).unwrap();

        assert_eq!(autosave.channels(), set_of(&["general"]));
    }

    #[test]
    fn turning_it_on_twice_keeps_one_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut autosave = autosave_at(&dir);

        autosave.set("general", true).unwrap();
        autosave.set("general", true).unwrap();

        assert_eq!(autosave.channels.len(), 1);
    }

    #[test]
    fn turning_it_off_removes_only_that_channel() {
        let dir = tempfile::tempdir().unwrap();
        let mut autosave = autosave_at(&dir);
        autosave.set("general", true).unwrap();
        autosave.set("project-x", true).unwrap();

        autosave.set("general", false).unwrap();

        assert_eq!(autosave.channels(), set_of(&["project-x"]));
    }

    #[test]
    fn turning_it_off_where_it_was_never_on_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let mut autosave = autosave_at(&dir);

        autosave.set("general", false).unwrap();

        assert!(
            !dir.path().join("autosave").exists(),
            "leaving a channel that never had auto-save must not create the file"
        );
    }

    #[test]
    fn the_channels_persist_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut autosave = autosave_at(&dir);
        autosave.set("general", true).unwrap();
        autosave.set("project-x", true).unwrap();
        autosave.set("general", false).unwrap();

        let reloaded = autosave_at(&dir);

        assert_eq!(reloaded.channels(), set_of(&["project-x"]));
    }

    #[test]
    fn the_folder_for_the_file_is_made_when_it_is_first_written() {
        let dir = tempfile::tempdir().unwrap();
        let mut autosave =
            AutoSave::load(dir.path().join("not").join("yet").join("autosave")).unwrap();

        autosave.set("general", true).unwrap();

        assert_eq!(autosave.channels(), set_of(&["general"]));
        assert!(
            dir.path()
                .join("not")
                .join("yet")
                .join("autosave")
                .is_file()
        );
    }
}
