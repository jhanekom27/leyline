//! Persisted receive-side channel-folder mirroring settings.
//!
//! This is deliberately only an opt-in list.  It never watches, publishes,
//! deletes, or renames local files.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use tracing::warn;

pub struct SyncSettings {
    path: PathBuf,
    channels: Vec<String>,
}

impl SyncSettings {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let channels = match fs::read(&path) {
            Ok(bytes) => postcard::from_bytes(&bytes).unwrap_or_else(|err| {
                warn!("dropping corrupt sync settings: {err}");
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
        self.channels.iter().cloned().collect()
    }

    pub fn set(&mut self, channel: &str, on: bool) -> anyhow::Result<()> {
        if self.channels.iter().any(|known| known == channel) == on {
            return Ok(());
        }
        if on {
            self.channels.push(channel.to_string());
        } else {
            self.channels.retain(|known| known != channel);
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes =
            postcard::to_stdvec(&self.channels).context("failed to encode sync settings")?;
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
        assert!(settings.channels().is_empty());

        settings.set("general", true).unwrap();
        settings.set("project", true).unwrap();
        settings.set("general", false).unwrap();

        assert_eq!(
            SyncSettings::load(path).unwrap().channels(),
            HashSet::from(["project".into()])
        );
    }

    #[test]
    fn corrupt_settings_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync");
        fs::write(&path, b"not postcard").unwrap();

        assert!(SyncSettings::load(path).unwrap().channels().is_empty());
    }
}
