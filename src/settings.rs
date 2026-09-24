//! Persisted local preferences -- currently just whether incoming messages
//! ring the terminal bell (`/bell`, see `app::AppState::run_bell`). Local
//! only, like `contacts.rs`; nothing here is ever sent over the wire.
//!
//! Mirrors `contacts.rs`/`channel_registry.rs`'s persisted-postcard-file
//! pattern: small, infrequently-changed data, so it's simplest to just
//! re-encode and rewrite the whole file on every change.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// The persisted shape itself, kept separate from `Settings` (which also
/// carries the file's `path`) so it can be encoded/decoded as one plain
/// value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SettingsData {
    /// Whether an incoming message rings the terminal bell -- on by
    /// default, since the whole point is to notice a message without
    /// having to watch the TUI.
    bell_enabled: bool,
}

impl Default for SettingsData {
    fn default() -> Self {
        Self { bell_enabled: true }
    }
}

/// Local, whole-file-rewrite-on-change store of user preferences.
///
/// Expected to stay tiny (a handful of booleans/flags at most), so like
/// `contacts.rs`, it's simplest to just re-encode and rewrite the whole
/// file on every change.
pub struct Settings {
    path: PathBuf,
    data: SettingsData,
}

impl Settings {
    /// Loads the settings persisted at `path`, or starts at defaults if the
    /// file doesn't exist yet.
    ///
    /// A corrupt file is logged and treated as defaults rather than failing
    /// startup -- like `contacts.rs`, losing it just means falling back to
    /// the bell's default-on behavior, not any real data loss. Only a
    /// genuine I/O error (e.g. permission denied) is returned as `Err`.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let data = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => SettingsData::default(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, data })
    }

    /// Whether incoming messages should ring the terminal bell -- seeded
    /// into `AppState` once at startup (see `AppState::load_settings`).
    pub fn bell_enabled(&self) -> bool {
        self.data.bell_enabled
    }

    /// Persists a new bell preference, overwriting whatever was recorded
    /// before (so re-running `/bell` flips it back and forth across
    /// restarts too).
    pub fn set_bell_enabled(&mut self, enabled: bool) -> anyhow::Result<()> {
        self.data.bell_enabled = enabled;
        self.save()
    }

    /// Re-encodes and rewrites the whole settings file.
    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes = postcard::to_stdvec(&self.data).context("failed to encode settings")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write settings to {}", self.path.display()))
    }
}

/// Decodes the settings file's contents, tolerating corruption by logging
/// and falling back to defaults -- see `Settings::load`.
fn decode(bytes: &[u8]) -> SettingsData {
    postcard::from_bytes(bytes).unwrap_or_else(|err| {
        warn!("dropping corrupt settings file: {err}");
        SettingsData::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_at(dir: &tempfile::TempDir) -> Settings {
        Settings::load(dir.path().join("settings")).unwrap()
    }

    #[test]
    fn missing_file_defaults_bell_to_enabled() {
        let dir = tempfile::tempdir().unwrap();
        assert!(settings_at(&dir).bell_enabled());
    }

    #[test]
    fn corrupt_file_is_tolerated_as_defaults() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings"), b"not a valid settings file").unwrap();
        assert!(settings_at(&dir).bell_enabled());
    }

    #[test]
    fn set_bell_enabled_then_bell_enabled_reflects_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = settings_at(&dir);
        settings.set_bell_enabled(false).unwrap();
        assert!(!settings.bell_enabled());
    }

    #[test]
    fn persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings");

        let mut settings = Settings::load(path.clone()).unwrap();
        settings.set_bell_enabled(false).unwrap();

        let reloaded = Settings::load(path).unwrap();
        assert!(!reloaded.bell_enabled());
    }

    #[test]
    fn toggling_back_persists_too() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings");

        let mut settings = Settings::load(path.clone()).unwrap();
        settings.set_bell_enabled(false).unwrap();
        settings.set_bell_enabled(true).unwrap();

        let reloaded = Settings::load(path).unwrap();
        assert!(reloaded.bell_enabled());
    }
}
