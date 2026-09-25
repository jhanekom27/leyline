//! Persisted local preferences: whether incoming messages ring the
//! terminal bell (`/bell`, see `app::AppState::run_bell`), and the last
//! broadcast nickname set via `/nick` (see `net::Net::set_nickname`, which
//! is what actually sends it to peers -- this module only remembers it
//! locally, so it doesn't need to be retyped every session). Local only,
//! like `contacts.rs`: loading/saving these preferences is never itself
//! network activity.
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
    /// The last broadcast nickname set via `/nick`, if any -- seeded back
    /// into `net::Net` at startup so it can re-announce it once a channel
    /// gains a neighbor (see `net::Net::announce_nickname`), the same way
    /// it already does mid-session. `None` until `/nick` is run for the
    /// first time.
    nickname: Option<String>,
}

impl Default for SettingsData {
    fn default() -> Self {
        Self {
            bell_enabled: true,
            nickname: None,
        }
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
    /// the bell's default-on behavior and an unset nickname, not any real
    /// data loss. Only a genuine I/O error (e.g. permission denied) is
    /// returned as `Err`.
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

    /// Our last broadcast nickname, if `/nick` has ever been run -- seeded
    /// into `net::Net` once at startup (see `net::Net::start`) so it can
    /// re-announce it once a channel gains a neighbor, without needing it
    /// retyped every session.
    pub fn nickname(&self) -> Option<String> {
        self.data.nickname.clone()
    }

    /// Persists a new broadcast nickname, overwriting whatever was recorded
    /// before (so re-running `/nick` updates what's restored next time).
    pub fn set_nickname(&mut self, nickname: String) -> anyhow::Result<()> {
        self.data.nickname = Some(nickname);
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

    #[test]
    fn missing_file_defaults_nickname_to_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(settings_at(&dir).nickname(), None);
    }

    #[test]
    fn corrupt_file_is_tolerated_as_no_nickname() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings"), b"not a valid settings file").unwrap();
        assert_eq!(settings_at(&dir).nickname(), None);
    }

    #[test]
    fn set_nickname_then_nickname_reflects_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = settings_at(&dir);
        settings.set_nickname("Alice".to_string()).unwrap();
        assert_eq!(settings.nickname(), Some("Alice".to_string()));
    }

    #[test]
    fn nickname_persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings");

        let mut settings = Settings::load(path.clone()).unwrap();
        settings.set_nickname("Alice".to_string()).unwrap();

        let reloaded = Settings::load(path).unwrap();
        assert_eq!(reloaded.nickname(), Some("Alice".to_string()));
    }

    #[test]
    fn nickname_and_bell_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = settings_at(&dir);
        settings.set_bell_enabled(false).unwrap();
        settings.set_nickname("Alice".to_string()).unwrap();
        assert!(!settings.bell_enabled());
        assert_eq!(settings.nickname(), Some("Alice".to_string()));
    }
}
