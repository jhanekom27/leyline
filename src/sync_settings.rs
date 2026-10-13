//! Persisted receive-side channel-folder mirroring settings.
//!
//! This is deliberately only an opt-in list.  It never watches, publishes,
//! deletes, or renames local files.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use crate::sync_state::SyncMode;
use anyhow::Context;
use tracing::warn;

pub struct SyncSettings {
    path: PathBuf,
    modes: HashMap<String, SyncMode>,
}

impl SyncSettings {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let modes = match fs::read(&path) {
            Ok(bytes) => postcard::from_bytes::<Vec<(String, SyncMode)>>(&bytes)
                .map(|entries| entries.into_iter().collect())
                // LEY-26.5 stored only enabled channels. Preserve those
                // settings as active while moving to explicit modes.
                .or_else(|_| {
                    postcard::from_bytes::<Vec<String>>(&bytes).map(|channels| {
                        channels
                            .into_iter()
                            .map(|channel| (channel, SyncMode::Active))
                            .collect()
                    })
                })
                .unwrap_or_else(|err| {
                    warn!("dropping corrupt sync settings: {err}");
                    HashMap::new()
                }),
            Err(err) if err.kind() == ErrorKind::NotFound => HashMap::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, modes })
    }

    pub fn modes(&self) -> HashMap<String, SyncMode> {
        self.modes.clone()
    }

    pub fn active_channels(&self) -> Vec<String> {
        self.modes
            .iter()
            .filter(|(_, mode)| mode.is_active())
            .map(|(channel, _)| channel.clone())
            .collect()
    }

    pub fn set(&mut self, channel: &str, mode: SyncMode) -> anyhow::Result<()> {
        if self.modes.get(channel).copied().unwrap_or_default() == mode {
            return Ok(());
        }
        self.modes.insert(channel.to_string(), mode);
        self.write()
    }

    pub fn forget(&mut self, channel: &str) -> anyhow::Result<()> {
        if self.modes.remove(channel).is_none() {
            return Ok(());
        }
        self.write()
    }

    fn write(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let mut entries: Vec<_> = self
            .modes
            .iter()
            .map(|(channel, mode)| (channel.clone(), *mode))
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let bytes = postcard::to_stdvec(&entries).context("failed to encode sync settings")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setting_persists_per_channel_and_defaults_to_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync");
        let mut settings = SyncSettings::load(path.clone()).unwrap();
        assert!(settings.modes().is_empty());

        settings.set("general", SyncMode::Active).unwrap();
        settings.set("project", SyncMode::Paused).unwrap();
        settings.set("general", SyncMode::Off).unwrap();

        assert_eq!(
            SyncSettings::load(path).unwrap().modes(),
            HashMap::from([
                ("general".into(), SyncMode::Off),
                ("project".into(), SyncMode::Paused),
            ])
        );
    }

    #[test]
    fn corrupt_settings_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync");
        fs::write(&path, b"not postcard").unwrap();

        assert!(SyncSettings::load(path).unwrap().modes().is_empty());
    }

    #[test]
    fn legacy_enabled_channels_load_as_active() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync");
        fs::write(
            &path,
            postcard::to_stdvec(&vec!["general".to_string()]).unwrap(),
        )
        .unwrap();

        assert_eq!(
            SyncSettings::load(path).unwrap().modes(),
            HashMap::from([("general".into(), SyncMode::Active)])
        );
    }
}
