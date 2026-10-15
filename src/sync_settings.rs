//! Persisted per-channel folder-sync configuration.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use crate::sync_state::{SyncConfig, SyncPolicy};
use anyhow::Context;
use serde::Deserialize;
use tracing::warn;

pub struct SyncSettings {
    path: PathBuf,
    configs: HashMap<String, SyncConfig>,
}

impl SyncSettings {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let configs = match fs::read(&path) {
            Ok(bytes) => postcard::from_bytes::<Vec<(String, SyncConfig)>>(&bytes)
                .map(|entries| {
                    entries
                        .into_iter()
                        .filter_map(|(channel, config)| {
                            if config.rules_are_valid() {
                                Some((channel, config))
                            } else {
                                warn!(%channel, "dropping sync setting with an invalid path rule");
                                None
                            }
                        })
                        .collect()
                })
                .or_else(|_| legacy_modes(&bytes))
                // LEY-26.5 stored only enabled channels. Preserve those
                // settings as publish-and-receive while adding policies.
                .or_else(|_| {
                    postcard::from_bytes::<Vec<String>>(&bytes).map(|channels| {
                        channels
                            .into_iter()
                            .map(|channel| {
                                (
                                    channel,
                                    SyncConfig {
                                        policy: SyncPolicy::PublishAndReceive,
                                        ..SyncConfig::default()
                                    },
                                )
                            })
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
        Ok(Self { path, configs })
    }

    pub fn configs(&self) -> HashMap<String, SyncConfig> {
        self.configs.clone()
    }

    pub fn publishing_channels(&self) -> Vec<String> {
        self.configs
            .iter()
            .filter(|(_, config)| config.publishes())
            .map(|(channel, _)| channel.clone())
            .collect()
    }

    pub fn allows_path(&self, channel: &str, path: &str) -> bool {
        self.configs
            .get(channel)
            .is_none_or(|config| config.allows_path(path))
    }

    pub fn set(&mut self, channel: &str, config: SyncConfig) -> anyhow::Result<()> {
        if self.configs.get(channel) == Some(&config) {
            return Ok(());
        }
        self.configs.insert(channel.to_string(), config);
        self.write()
    }

    pub fn forget(&mut self, channel: &str) -> anyhow::Result<()> {
        if self.configs.remove(channel).is_none() {
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
            .configs
            .iter()
            .map(|(channel, config)| (channel.clone(), config.clone()))
            .collect();
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let bytes = postcard::to_stdvec(&entries).context("failed to encode sync settings")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }
}

#[derive(Deserialize, serde::Serialize)]
enum LegacySyncMode {
    Off,
    Active,
    Paused,
}

fn legacy_modes(bytes: &[u8]) -> Result<HashMap<String, SyncConfig>, postcard::Error> {
    postcard::from_bytes::<Vec<(String, LegacySyncMode)>>(bytes).map(|entries| {
        entries
            .into_iter()
            .map(|(channel, mode)| {
                let (policy, paused) = match mode {
                    LegacySyncMode::Off => (SyncPolicy::Off, false),
                    LegacySyncMode::Active => (SyncPolicy::PublishAndReceive, false),
                    LegacySyncMode::Paused => (SyncPolicy::PublishAndReceive, true),
                };
                (
                    channel,
                    SyncConfig {
                        policy,
                        paused,
                        rules: Vec::new(),
                    },
                )
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_state::SyncRule;

    #[test]
    fn setting_persists_per_channel_and_defaults_to_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync");
        let mut settings = SyncSettings::load(path.clone()).unwrap();
        assert!(settings.configs().is_empty());

        settings
            .set(
                "general",
                SyncConfig {
                    policy: SyncPolicy::ReceiveOnly,
                    rules: vec![SyncRule::include("docs/**").unwrap()],
                    ..SyncConfig::default()
                },
            )
            .unwrap();
        settings
            .set(
                "project",
                SyncConfig {
                    policy: SyncPolicy::PublishAndReceive,
                    paused: true,
                    ..SyncConfig::default()
                },
            )
            .unwrap();

        assert_eq!(
            SyncSettings::load(path).unwrap().configs(),
            HashMap::from([
                (
                    "general".into(),
                    SyncConfig {
                        policy: SyncPolicy::ReceiveOnly,
                        rules: vec![SyncRule::include("docs/**").unwrap()],
                        ..SyncConfig::default()
                    },
                ),
                (
                    "project".into(),
                    SyncConfig {
                        policy: SyncPolicy::PublishAndReceive,
                        paused: true,
                        ..SyncConfig::default()
                    },
                ),
            ])
        );
    }

    #[test]
    fn corrupt_settings_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync");
        fs::write(&path, b"not postcard").unwrap();

        assert!(SyncSettings::load(path).unwrap().configs().is_empty());
    }

    #[test]
    fn legacy_enabled_channels_load_as_publish_and_receive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync");
        fs::write(
            &path,
            postcard::to_stdvec(&vec!["general".to_string()]).unwrap(),
        )
        .unwrap();

        assert_eq!(
            SyncSettings::load(path).unwrap().configs(),
            HashMap::from([(
                "general".into(),
                SyncConfig {
                    policy: SyncPolicy::PublishAndReceive,
                    ..SyncConfig::default()
                },
            )])
        );
    }

    #[test]
    fn legacy_modes_preserve_active_and_paused_behavior() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync");
        fs::write(
            &path,
            postcard::to_stdvec(&vec![
                ("active".to_string(), LegacySyncMode::Active),
                ("paused".to_string(), LegacySyncMode::Paused),
            ])
            .unwrap(),
        )
        .unwrap();

        let configs = SyncSettings::load(path).unwrap().configs();
        assert_eq!(configs["active"].policy, SyncPolicy::PublishAndReceive);
        assert!(!configs["active"].paused);
        assert!(configs["paused"].paused);
    }

    #[test]
    fn only_unpaused_publish_and_receive_channels_are_scanned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync");
        let mut settings = SyncSettings::load(path).unwrap();
        settings
            .set(
                "publish",
                SyncConfig {
                    policy: SyncPolicy::PublishAndReceive,
                    ..SyncConfig::default()
                },
            )
            .unwrap();
        settings
            .set(
                "receive",
                SyncConfig {
                    policy: SyncPolicy::ReceiveOnly,
                    ..SyncConfig::default()
                },
            )
            .unwrap();
        settings
            .set(
                "paused",
                SyncConfig {
                    policy: SyncPolicy::PublishAndReceive,
                    paused: true,
                    ..SyncConfig::default()
                },
            )
            .unwrap();

        assert_eq!(settings.publishing_channels(), vec!["publish"]);
    }
}
