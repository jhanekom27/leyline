//! Persisted set of device ids a user has explicitly chosen to stop
//! trusting, via `/forget-device` -- see concept.md's "Identity &
//! channels". Consulted when resolving a device to its canonical user id
//! (`app::AppState::canonical_id` treats a forgotten device as if no
//! certificate were known for it) and before acting on a
//! `crate::message::ChannelSyncAnnounce` received from a forgotten sender
//! (see main.rs), so a lost or compromised device stops being treated as
//! one of your own once you've said so locally.
//!
//! This is local and best-effort, not real revocation: a forgotten device
//! still physically holds the shared user key and can still sign valid
//! certificates and broadcast on the device-sync channel -- forgetting it
//! only changes what *this* install chooses to act on. Real removal is
//! the same "rotate the secret" story LEY-11 already documents for
//! channels.
//!
//! Mirrors `contacts.rs`'s persisted-postcard-file pattern: small,
//! infrequently-changed data, so it's simplest to just re-encode and
//! rewrite the whole file on every change.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use tracing::warn;

/// Local, whole-file-rewrite-on-change store of forgotten device ids.
pub struct ForgottenDevices {
    path: PathBuf,
    ids: HashSet<[u8; 32]>,
}

impl ForgottenDevices {
    /// Loads the set persisted at `path`, or starts empty if the file
    /// doesn't exist yet.
    ///
    /// A corrupt file is logged and treated as empty rather than failing
    /// startup -- like `contacts.rs`, losing it just means a previously
    /// forgotten device is trusted again until re-forgotten, not any real
    /// data loss. Only a genuine I/O error (e.g. permission denied) is
    /// returned as `Err`.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let ids = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => HashSet::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, ids })
    }

    /// Every forgotten device id, seeded into `AppState` once at startup
    /// (see `AppState::load_forgotten_devices`).
    pub fn all(&self) -> HashSet<[u8; 32]> {
        self.ids.clone()
    }

    pub fn is_forgotten(&self, id: &[u8; 32]) -> bool {
        self.ids.contains(id)
    }

    /// Marks `id` as forgotten. A no-op (including no write to disk) if
    /// already forgotten.
    pub fn forget(&mut self, id: [u8; 32]) -> anyhow::Result<()> {
        if !self.ids.insert(id) {
            return Ok(());
        }
        self.save()
    }

    /// Re-encodes and rewrites the whole file.
    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let ids: Vec<[u8; 32]> = self.ids.iter().copied().collect();
        let bytes = postcard::to_stdvec(&ids).context("failed to encode forgotten devices")?;
        fs::write(&self.path, bytes).with_context(|| {
            format!(
                "failed to write forgotten devices to {}",
                self.path.display()
            )
        })
    }
}

/// Decodes the file's contents, tolerating corruption by logging and
/// falling back to an empty set -- see `ForgottenDevices::load`.
fn decode(bytes: &[u8]) -> HashSet<[u8; 32]> {
    postcard::from_bytes::<Vec<[u8; 32]>>(bytes)
        .map(|ids| ids.into_iter().collect())
        .unwrap_or_else(|err| {
            warn!("dropping corrupt forgotten-devices file: {err}");
            HashSet::new()
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE_DEVICE: [u8; 32] = [1; 32];
    const BOB_DEVICE: [u8; 32] = [2; 32];

    fn store_at(dir: &tempfile::TempDir) -> ForgottenDevices {
        ForgottenDevices::load(dir.path().join("forgotten")).unwrap()
    }

    #[test]
    fn missing_file_is_an_empty_set() {
        let dir = tempfile::tempdir().unwrap();
        assert!(store_at(&dir).all().is_empty());
    }

    #[test]
    fn corrupt_file_is_tolerated_as_an_empty_set() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("forgotten"), b"not a valid set").unwrap();
        assert!(store_at(&dir).all().is_empty());
    }

    #[test]
    fn forget_then_is_forgotten_returns_true() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_at(&dir);
        assert!(!store.is_forgotten(&ALICE_DEVICE));
        store.forget(ALICE_DEVICE).unwrap();
        assert!(store.is_forgotten(&ALICE_DEVICE));
    }

    #[test]
    fn forgetting_again_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_at(&dir);
        store.forget(ALICE_DEVICE).unwrap();
        store.forget(ALICE_DEVICE).unwrap();
        assert_eq!(store.all(), HashSet::from([ALICE_DEVICE]));
    }

    #[test]
    fn forgetting_one_device_does_not_affect_another() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = store_at(&dir);
        store.forget(ALICE_DEVICE).unwrap();
        assert!(!store.is_forgotten(&BOB_DEVICE));
    }

    #[test]
    fn persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("forgotten");

        let mut store = ForgottenDevices::load(path.clone()).unwrap();
        store.forget(ALICE_DEVICE).unwrap();

        let reloaded = ForgottenDevices::load(path).unwrap();
        assert!(reloaded.is_forgotten(&ALICE_DEVICE));
    }
}
