//! Persisted record of the files `/save` has written to disk: which channel's
//! copy of which file went where -- what the `/files` overlay's check mark
//! and its open key are built on (`app::AppState::viewing_files`). Nothing
//! else knows: `NetEvent::FileSaved` is only a notice, and the blob store
//! (`backfill.rs`) serves content by hash, not by wherever someone put a copy.
//!
//! Keyed by channel as well as by file, because every channel saves into a
//! folder of its own (`files::channel_dir`): a file saved from `#general` is
//! not saved in `#random`'s folder, so `#random`'s overlay must still offer
//! to save it there.
//!
//! Mirrors `dm_registry.rs`'s persisted-postcard-file pattern: local only,
//! never sent over the wire, the whole file re-encoded and rewritten on every
//! change. Unlike the registries of channels, DMs or names, this one grows by
//! a small record for every file ever saved instead of staying a handful --
//! still far too little for a rewrite per save to matter.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::Context;
use iroh_blobs::Hash;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// One saved copy: the stored name of the channel it was saved for (the key
/// of its folder -- see `files::channel_dir`), the content hash of the file
/// (`message::FileAttachment::hash`), and where the copy was put.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedRecord {
    channel: String,
    hash: Hash,
    /// Absolute, and UTF-8 -- `SavedFiles::record` refuses anything else, so
    /// a record can always be written down and a path read back out of it is
    /// always one that can be opened.
    path: String,
}

/// Local, whole-file-rewrite-on-change store of where `/save` put each
/// channel's files.
pub struct SavedFiles {
    path: PathBuf,
    records: Vec<SavedRecord>,
}

impl SavedFiles {
    /// Loads the records persisted at `path`, or starts empty if the file
    /// doesn't exist yet.
    ///
    /// A corrupt file is logged and treated as empty rather than failing
    /// startup -- losing it just means the files overlay shows everything
    /// unsaved until it is saved again, and the files themselves are
    /// untouched. Only a genuine I/O error (e.g. permission denied) is
    /// returned as `Err`.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let records = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, records })
    }

    /// Records that `channel`'s copy of the file `hash` is at `copy`,
    /// replacing whatever was recorded for that file in that channel before
    /// (a file saved a second time is only known by its newest copy).
    ///
    /// `copy` must be an absolute path that is valid UTF-8 -- anything else
    /// is refused here, once, so nothing downstream has to wonder.
    pub fn record(&mut self, channel: &str, hash: Hash, copy: &Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            copy.is_absolute(),
            "{} is not an absolute path",
            copy.display()
        );
        let copy = copy
            .to_str()
            .with_context(|| format!("{} is not valid UTF-8", copy.display()))?
            .to_string();
        match self
            .records
            .iter_mut()
            .find(|record| record.channel == channel && record.hash == hash)
        {
            Some(existing) => existing.path = copy,
            None => self.records.push(SavedRecord {
                channel: channel.to_string(),
                hash,
                path: copy,
            }),
        }
        self.save()
    }

    /// The files saved for `channel` that are still on disk, by content hash,
    /// each with where its copy is. Asks the filesystem every time: a copy
    /// that has been deleted -- or moved away -- is simply not in the answer,
    /// and is again if it comes back. Nothing is forgotten for being absent,
    /// since a drive that happens to be unmounted right now must not erase
    /// what is known about what it holds.
    pub fn present_in(&self, channel: &str) -> HashMap<Hash, PathBuf> {
        self.records
            .iter()
            .filter(|record| record.channel == channel)
            .map(|record| (record.hash, PathBuf::from(&record.path)))
            .filter(|(_, path)| path.is_file())
            .collect()
    }

    /// Re-encodes and rewrites the whole records file.
    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes = postcard::to_stdvec(&self.records).context("failed to encode saved files")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write saved files to {}", self.path.display()))
    }
}

/// Decodes the records file's contents, tolerating corruption by logging and
/// falling back to an empty list -- see `SavedFiles::load`.
fn decode(bytes: &[u8]) -> Vec<SavedRecord> {
    postcard::from_bytes(bytes).unwrap_or_else(|err| {
        warn!("dropping corrupt saved files record: {err}");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved_at(dir: &tempfile::TempDir) -> SavedFiles {
        SavedFiles::load(dir.path().join("saved_files")).unwrap()
    }

    /// Writes a small file called `name` into `dir` and returns its path.
    fn copy_in(dir: &tempfile::TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, b"contents").unwrap();
        path
    }

    fn hash(content: &[u8]) -> Hash {
        Hash::new(content)
    }

    #[test]
    fn missing_file_is_an_empty_record() {
        let dir = tempfile::tempdir().unwrap();
        assert!(saved_at(&dir).present_in("general").is_empty());
    }

    #[test]
    fn corrupt_file_is_tolerated_as_an_empty_record() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("saved_files"), b"not a valid record").unwrap();
        assert!(saved_at(&dir).present_in("general").is_empty());
    }

    #[test]
    fn a_recorded_copy_is_present_by_its_hash_with_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let copy = copy_in(&dir, "report.pdf");
        let mut saved = saved_at(&dir);

        saved.record("general", hash(b"report"), &copy).unwrap();

        assert_eq!(
            saved.present_in("general"),
            HashMap::from([(hash(b"report"), copy)])
        );
    }

    #[test]
    fn recording_a_file_again_replaces_its_copy_instead_of_adding_one() {
        let dir = tempfile::tempdir().unwrap();
        let first = copy_in(&dir, "report.pdf");
        let second = copy_in(&dir, "report (1).pdf");
        let mut saved = saved_at(&dir);
        saved.record("general", hash(b"report"), &first).unwrap();

        saved.record("general", hash(b"report"), &second).unwrap();

        assert_eq!(
            saved.present_in("general"),
            HashMap::from([(hash(b"report"), second)]),
            "the newest copy is the one that counts"
        );
    }

    #[test]
    fn channels_are_independent_even_for_the_same_file() {
        let dir = tempfile::tempdir().unwrap();
        let in_general = copy_in(&dir, "general-copy.pdf");
        let mut saved = saved_at(&dir);
        saved
            .record("general", hash(b"report"), &in_general)
            .unwrap();

        assert!(
            saved.present_in("random").is_empty(),
            "a copy in #general's folder is not one in #random's"
        );
        let in_random = copy_in(&dir, "random-copy.pdf");
        saved.record("random", hash(b"report"), &in_random).unwrap();
        assert_eq!(
            saved.present_in("general"),
            HashMap::from([(hash(b"report"), in_general)])
        );
        assert_eq!(
            saved.present_in("random"),
            HashMap::from([(hash(b"report"), in_random)])
        );
    }

    #[test]
    fn one_channels_files_are_told_apart_by_hash() {
        let dir = tempfile::tempdir().unwrap();
        let a = copy_in(&dir, "a.pdf");
        let b = copy_in(&dir, "b.pdf");
        let mut saved = saved_at(&dir);
        saved.record("general", hash(b"a"), &a).unwrap();
        saved.record("general", hash(b"b"), &b).unwrap();

        assert_eq!(
            saved.present_in("general"),
            HashMap::from([(hash(b"a"), a), (hash(b"b"), b)])
        );
    }

    #[test]
    fn a_deleted_copy_is_not_present_and_is_again_once_it_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let copy = copy_in(&dir, "report.pdf");
        let mut saved = saved_at(&dir);
        saved.record("general", hash(b"report"), &copy).unwrap();

        fs::remove_file(&copy).unwrap();
        assert!(saved.present_in("general").is_empty());

        fs::write(&copy, b"restored").unwrap();
        assert_eq!(
            saved.present_in("general"),
            HashMap::from([(hash(b"report"), copy)]),
            "nothing is forgotten for being absent"
        );
    }

    #[test]
    fn a_folder_where_the_copy_was_is_not_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("report.pdf");
        fs::create_dir(&copy).unwrap();
        let mut saved = saved_at(&dir);
        saved.record("general", hash(b"report"), &copy).unwrap();

        assert!(saved.present_in("general").is_empty());
    }

    #[test]
    fn records_persist_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("saved_files");
        let copy = copy_in(&dir, "report.pdf");
        let mut saved = SavedFiles::load(path.clone()).unwrap();
        saved.record("general", hash(b"report"), &copy).unwrap();

        let reloaded = SavedFiles::load(path).unwrap();

        assert_eq!(
            reloaded.present_in("general"),
            HashMap::from([(hash(b"report"), copy)])
        );
    }

    #[test]
    fn a_relative_path_is_refused_and_nothing_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let mut saved = saved_at(&dir);

        let err = saved
            .record("general", hash(b"report"), Path::new("relative/report.pdf"))
            .unwrap_err();

        assert!(err.to_string().contains("not an absolute path"), "{err}");
        assert!(saved.records.is_empty());
        assert!(
            !dir.path().join("saved_files").exists(),
            "a refused record must not create the file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_utf8_is_refused_and_nothing_is_recorded() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let dir = tempfile::tempdir().unwrap();
        let mut saved = saved_at(&dir);
        let copy = Path::new(OsStr::from_bytes(b"/tmp/not-utf8-\xff.pdf"));

        let err = saved.record("general", hash(b"report"), copy).unwrap_err();

        assert!(err.to_string().contains("not valid UTF-8"), "{err}");
        assert!(saved.records.is_empty());
    }
}
