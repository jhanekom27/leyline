//! Durable file-share metadata, independent from bounded chat scrollback.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::message::{ChatMessage, FileAttachment};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedFile {
    pub message_id: u64,
    pub file: FileAttachment,
    pub sender: [u8; 32],
    pub ts_unix_ms: u64,
}

#[derive(Serialize, Deserialize)]
struct Record {
    channel: String,
    shared: SharedFile,
}

pub struct SharedFiles {
    path: PathBuf,
    records: Vec<Record>,
}

impl SharedFiles {
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let records = match fs::read(&path) {
            Ok(bytes) => postcard::from_bytes(&bytes).unwrap_or_else(|err| {
                warn!("dropping corrupt shared-file index: {err}");
                Vec::new()
            }),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, records })
    }

    pub fn record_message(&mut self, channel: &str, message: &ChatMessage) -> anyhow::Result<()> {
        let Some(file) = &message.attachment else {
            return Ok(());
        };
        if self
            .records
            .iter()
            .any(|record| record.channel == channel && record.shared.message_id == message.id)
        {
            return Ok(());
        }
        self.records.push(Record {
            channel: channel.to_string(),
            shared: SharedFile {
                message_id: message.id,
                file: file.clone(),
                sender: message.sender,
                ts_unix_ms: message.ts_unix_ms,
            },
        });
        self.save()
    }

    pub fn record_messages(
        &mut self,
        channel: &str,
        messages: &[ChatMessage],
    ) -> anyhow::Result<()> {
        let before = self.records.len();
        for message in messages {
            let Some(file) = &message.attachment else {
                continue;
            };
            if self
                .records
                .iter()
                .any(|record| record.channel == channel && record.shared.message_id == message.id)
            {
                continue;
            }
            self.records.push(Record {
                channel: channel.to_string(),
                shared: SharedFile {
                    message_id: message.id,
                    file: file.clone(),
                    sender: message.sender,
                    ts_unix_ms: message.ts_unix_ms,
                },
            });
        }
        if self.records.len() != before {
            self.save()?;
        }
        Ok(())
    }

    pub fn snapshot(&self) -> HashMap<String, Vec<SharedFile>> {
        let mut files: HashMap<String, Vec<SharedFile>> = HashMap::new();
        for record in &self.records {
            files
                .entry(record.channel.clone())
                .or_default()
                .push(record.shared.clone());
        }
        for entries in files.values_mut() {
            entries.sort_by_key(|shared| std::cmp::Reverse((shared.ts_unix_ms, shared.message_id)));
        }
        files
    }

    pub fn forget_channel(&mut self, channel: &str) -> anyhow::Result<()> {
        let before = self.records.len();
        self.records.retain(|record| record.channel != channel);
        if self.records.len() != before {
            self.save()?;
        }
        Ok(())
    }

    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&self.path, postcard::to_stdvec(&self.records)?)
            .with_context(|| format!("failed to write {}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_message() -> ChatMessage {
        ChatMessage {
            v: 3,
            id: 7,
            sender: [4; 32],
            ts_unix_ms: 123,
            text: String::new(),
            attachment: Some(FileAttachment {
                filename: "old.pdf".into(),
                size: 3,
                hash: iroh_blobs::Hash::new(b"old"),
            }),
            reply_to: None,
        }
    }

    #[test]
    fn indexed_files_survive_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared_files");
        let mut files = SharedFiles::load(path.clone()).unwrap();
        files.record_message("general", &file_message()).unwrap();

        let restored = SharedFiles::load(path).unwrap().snapshot();

        assert_eq!(restored["general"][0].file.filename, "old.pdf");
        assert_eq!(restored["general"][0].message_id, 7);
    }
}
