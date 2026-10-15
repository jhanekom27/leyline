//! Bounded local history of sync outcomes. Never sent to peers.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::warn;

pub const MAX_ACTIVITY_PER_CHANNEL: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activity {
    pub channel: String,
    pub path: String,
    pub action: String,
    pub outcome: String,
    pub at_unix_ms: u64,
}

pub struct SyncActivity {
    path: PathBuf,
    entries: Vec<Activity>,
}

impl SyncActivity {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let entries = match fs::read(&path) {
            Ok(bytes) => postcard::from_bytes(&bytes).unwrap_or_else(|err| {
                warn!("dropping corrupt sync activity: {err}");
                Vec::new()
            }),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, entries })
    }

    pub fn record(&mut self, entry: Activity) -> anyhow::Result<()> {
        if entry.action == "publish"
            && entry.outcome == "queued"
            && self.entries.iter().any(|known| {
                known.channel == entry.channel
                    && known.path == entry.path
                    && known.action == "publish"
                    && known.outcome == "queued"
            })
        {
            return Ok(());
        }
        if self.entries.last().is_some_and(|last| {
            last.channel == entry.channel
                && last.path == entry.path
                && last.action == entry.action
                && last.outcome == entry.outcome
        }) {
            return Ok(());
        }
        self.entries.push(entry);
        let channel = self.entries.last().expect("just pushed").channel.clone();
        while self
            .entries
            .iter()
            .filter(|entry| entry.channel == channel)
            .count()
            > MAX_ACTIVITY_PER_CHANNEL
        {
            let oldest = self
                .entries
                .iter()
                .position(|entry| entry.channel == channel)
                .expect("count above zero");
            self.entries.remove(oldest);
        }
        self.save()
    }

    pub fn recent(&self, channel: &str) -> Vec<Activity> {
        self.entries
            .iter()
            .rev()
            .filter(|entry| entry.channel == channel)
            .cloned()
            .collect()
    }

    pub fn forget(&mut self, channel: &str) -> anyhow::Result<()> {
        self.entries.retain(|entry| entry.channel != channel);
        self.save()
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&self.path, postcard::to_stdvec(&self.entries)?)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_channel_scoped_persisted_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync_activity");
        let mut history = SyncActivity::load(path.clone()).unwrap();
        for at in 0..=MAX_ACTIVITY_PER_CHANNEL {
            history
                .record(Activity {
                    channel: "general".into(),
                    path: format!("file-{at}"),
                    action: "download".into(),
                    outcome: "ok".into(),
                    at_unix_ms: at as u64,
                })
                .unwrap();
        }
        history
            .record(Activity {
                channel: "random".into(),
                path: "other".into(),
                action: "skip".into(),
                outcome: "filtered".into(),
                at_unix_ms: 1,
            })
            .unwrap();
        assert_eq!(history.recent("general").len(), MAX_ACTIVITY_PER_CHANNEL);
        assert_eq!(SyncActivity::load(path).unwrap().recent("random").len(), 1);
    }

    #[test]
    fn identical_consecutive_outcomes_are_coalesced_but_changes_remain() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = SyncActivity::load(dir.path().join("sync_activity")).unwrap();
        let queued = || Activity {
            channel: "general".into(),
            path: "report.txt".into(),
            action: "publish".into(),
            outcome: "queued".into(),
            at_unix_ms: 1,
        };

        history.record(queued()).unwrap();
        history.record(queued()).unwrap();
        history
            .record(Activity {
                outcome: "applied".into(),
                at_unix_ms: 2,
                ..queued()
            })
            .unwrap();

        assert_eq!(history.recent("general").len(), 2);
    }

    #[test]
    fn queued_publish_is_coalesced_across_interleaved_activity() {
        let dir = tempfile::tempdir().unwrap();
        let mut history = SyncActivity::load(dir.path().join("sync_activity")).unwrap();
        let queued = || Activity {
            channel: "general".into(),
            path: "report.txt".into(),
            action: "publish".into(),
            outcome: "queued".into(),
            at_unix_ms: 1,
        };

        history.record(queued()).unwrap();
        history
            .record(Activity {
                channel: "general".into(),
                path: "other.txt".into(),
                action: "download".into(),
                outcome: "applied".into(),
                at_unix_ms: 2,
            })
            .unwrap();
        history.record(queued()).unwrap();

        assert_eq!(history.recent("general").len(), 2);
    }
}
