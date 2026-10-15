//! Persisted auto-save settings: which channels have it on (`/autosave`) and
//! the finite device-wide limit for automatic downloads (`/autosave limit`).
//! Both are local only, never sent over the wire.
//!
//! Keyed by a channel's stored name, as its folder is
//! (`files::channel_dir`), so it is forgotten when the channel is left
//! (`AutoSave::set(.., false)`): a ticket's name is chosen by whoever shared
//! it, and a setting that outlived its channel would switch downloads on for
//! a different room that happens to reuse the name, which nobody asked for.
//! Pairing wipes the channel file for the same reason (main.rs's
//! `paths_to_wipe_for_pairing`), but leaves the device-wide limit alone.
//!
//! Mirrors `channel_renames.rs`'s persisted-postcard-file pattern: tiny and
//! rarely changed, so the whole file is simply rewritten on every change.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::warn;

const LIMIT_FILE: &str = "autosave_limit";
pub const DEFAULT_AUTOSAVE_LIMIT: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutoSaveLimit {
    Default,
    Bytes(u64),
}

impl AutoSaveLimit {
    pub fn max_bytes(self) -> u64 {
        match self {
            Self::Default => DEFAULT_AUTOSAVE_LIMIT,
            Self::Bytes(bytes) => bytes,
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("default") {
            return Ok(Self::Default);
        }
        let malformed = || {
            format!(
                "can't read '{text}' as an auto-save limit -- give a number and a unit (B, KB, MB, GB or TB), like 750MB or 2GB, or say default"
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
                "an auto-save limit of '{text}' would refuse every file -- /autosave limit <size> sets a real one, /autosave limit default restores the default"
            ));
        }
        if bytes >= u64::MAX as f64 {
            return Err(format!(
                "'{text}' is too big to count -- /autosave limit <size> sets a finite limit instead"
            ));
        }
        Ok(Self::Bytes(bytes as u64))
    }
}

pub fn limit(data_dir: &std::path::Path) -> anyhow::Result<AutoSaveLimit> {
    let path = data_dir.join(LIMIT_FILE);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(AutoSaveLimit::Default),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    Ok(AutoSaveLimit::parse(&text).unwrap_or_else(|reason| {
        warn!("ignoring {}: {reason}", path.display());
        AutoSaveLimit::Default
    }))
}

#[allow(dead_code)]
pub fn set_limit(data_dir: &std::path::Path, limit: AutoSaveLimit) -> Result<(), String> {
    let path = data_dir.join(LIMIT_FILE);
    let recorded = match limit {
        AutoSaveLimit::Default => match fs::remove_file(&path) {
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            other => other,
        },
        AutoSaveLimit::Bytes(bytes) => fs::write(&path, format!("{bytes}B")),
    };
    recorded.map_err(|err| format!("can't remember the auto-save limit: {err}"))
}

/// Local, whole-file-rewrite-on-change store of the channels with auto-save
/// on.
pub struct AutoSave {
    path: PathBuf,
    /// Stored channel names, each once.
    channels: Vec<String>,
    limits: HashMap<String, AutoSaveLimit>,
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
        let (channels, limits) = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => (Vec::new(), HashMap::new()),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self {
            path,
            channels,
            limits,
        })
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
        if self.channels.iter().any(|known| known == channel) == on
            && (on || !self.limits.contains_key(channel))
        {
            return Ok(());
        }
        if on {
            self.channels.push(channel.to_string());
        } else {
            self.channels.retain(|known| known != channel);
            self.limits.remove(channel);
        }
        self.save()
    }

    pub fn limits(&self) -> HashMap<String, AutoSaveLimit> {
        self.limits.clone()
    }
    pub fn set_channel_limit(
        &mut self,
        channel: &str,
        limit: Option<AutoSaveLimit>,
    ) -> anyhow::Result<()> {
        match limit {
            Some(limit) => {
                self.limits.insert(channel.into(), limit);
            }
            None => {
                self.limits.remove(channel);
            }
        }
        self.save()
    }

    /// Re-encodes and rewrites the whole file.
    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes = postcard::to_stdvec(&(self.channels.clone(), self.limits.clone()))
            .context("failed to encode auto-save")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write auto-save to {}", self.path.display()))
    }
}

/// Decodes the file's contents, tolerating corruption by logging and falling
/// back to an empty list -- see `AutoSave::load`.
fn decode(bytes: &[u8]) -> (Vec<String>, HashMap<String, AutoSaveLimit>) {
    postcard::from_bytes(bytes)
        .or_else(|_| {
            postcard::from_bytes::<Vec<String>>(bytes).map(|channels| (channels, HashMap::new()))
        })
        .unwrap_or_else(|err| {
            warn!("dropping corrupt auto-save file: {err}");
            (Vec::new(), HashMap::new())
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

    #[test]
    fn auto_save_limit_persists_a_finite_size_and_default_forgets_it() {
        let data = tempfile::tempdir().unwrap();
        set_limit(data.path(), AutoSaveLimit::Bytes(3 << 20)).unwrap();
        assert_eq!(limit(data.path()).unwrap(), AutoSaveLimit::Bytes(3 << 20));

        set_limit(data.path(), AutoSaveLimit::Default).unwrap();
        assert_eq!(limit(data.path()).unwrap(), AutoSaveLimit::Default);
        assert!(!data.path().join(LIMIT_FILE).exists());
    }

    #[test]
    fn auto_save_limit_parses_maxsize_syntax_but_refuses_off() {
        assert_eq!(
            AutoSaveLimit::parse("1.5 GB"),
            Ok(AutoSaveLimit::Bytes(3 << 29))
        );
        assert_eq!(AutoSaveLimit::parse("default"), Ok(AutoSaveLimit::Default));
        assert!(AutoSaveLimit::parse("off").is_err());
    }

    #[test]
    fn channel_limits_persist_independently_and_clear_with_channel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("autosave");
        let mut autosave = AutoSave::load(path.clone()).unwrap();
        autosave
            .set_channel_limit("general", Some(AutoSaveLimit::Bytes(1 << 20)))
            .unwrap();
        autosave
            .set_channel_limit("project", Some(AutoSaveLimit::Bytes(2 << 20)))
            .unwrap();
        autosave.set("general", false).unwrap();

        let limits = AutoSave::load(path).unwrap().limits();
        assert_eq!(limits.get("project"), Some(&AutoSaveLimit::Bytes(2 << 20)));
        assert!(!limits.contains_key("general"));
    }
}
