//! Persisted local pet names for peers: a mapping from endpoint id to a
//! name you've assigned yourself, never sent over the wire -- see
//! features.md's "Local petnames" and `AppState::display_name`.
//!
//! Mirrors `channel_registry.rs`'s persisted-postcard-file pattern: small,
//! infrequently-changed data, so it's simplest to just re-encode and
//! rewrite the whole file on every change.

use std::collections::HashMap;
use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// One assigned pet name: the endpoint id it names, and the name itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContactRecord {
    id: [u8; 32],
    name: String,
}

/// Local, whole-file-rewrite-on-change store of assigned pet names.
///
/// Expected to stay small (a handful of contacts), so like
/// `channel_registry.rs`, it's simplest to just re-encode and rewrite the
/// whole file on every change.
pub struct Contacts {
    path: PathBuf,
    contacts: Vec<ContactRecord>,
}

impl Contacts {
    /// Loads the contacts persisted at `path`, or starts empty if the file
    /// doesn't exist yet.
    ///
    /// A corrupt file is logged and treated as empty rather than failing
    /// startup -- like `channel_registry.rs`, losing it just means peers
    /// go back to showing their hex id until re-aliased, not any real data
    /// loss. Only a genuine I/O error (e.g. permission denied) is returned
    /// as `Err`.
    pub fn load(path: PathBuf) -> anyhow::Result<Self> {
        let contacts = match fs::read(&path) {
            Ok(bytes) => decode(&bytes),
            Err(err) if err.kind() == ErrorKind::NotFound => Vec::new(),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        Ok(Self { path, contacts })
    }

    /// All known pet names, keyed by endpoint id -- seeded into `AppState`
    /// once at startup (see `AppState::load_contacts`).
    pub fn all(&self) -> HashMap<[u8; 32], String> {
        self.contacts
            .iter()
            .map(|c| (c.id, c.name.clone()))
            .collect()
    }

    /// Assigns `name` to `id`, overwriting any pet name already recorded
    /// for it (so re-running `/alias` on the same id renames it).
    pub fn set(&mut self, id: [u8; 32], name: String) -> anyhow::Result<()> {
        match self.contacts.iter_mut().find(|c| c.id == id) {
            Some(existing) => existing.name = name,
            None => self.contacts.push(ContactRecord { id, name }),
        }
        self.save()
    }

    /// Re-encodes and rewrites the whole contacts file.
    fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let bytes = postcard::to_stdvec(&self.contacts).context("failed to encode contacts")?;
        fs::write(&self.path, bytes)
            .with_context(|| format!("failed to write contacts to {}", self.path.display()))
    }
}

/// Decodes the contacts file's contents, tolerating corruption by logging
/// and falling back to an empty list -- see `Contacts::load`.
fn decode(bytes: &[u8]) -> Vec<ContactRecord> {
    postcard::from_bytes(bytes).unwrap_or_else(|err| {
        warn!("dropping corrupt contacts file: {err}");
        Vec::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALICE: [u8; 32] = [1; 32];
    const BOB: [u8; 32] = [2; 32];

    fn contacts_at(dir: &tempfile::TempDir) -> Contacts {
        Contacts::load(dir.path().join("contacts")).unwrap()
    }

    #[test]
    fn missing_file_is_an_empty_contact_list() {
        let dir = tempfile::tempdir().unwrap();
        assert!(contacts_at(&dir).all().is_empty());
    }

    #[test]
    fn corrupt_file_is_tolerated_as_an_empty_contact_list() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("contacts"), b"not a valid contacts file").unwrap();
        assert!(contacts_at(&dir).all().is_empty());
    }

    #[test]
    fn set_then_all_contains_it() {
        let dir = tempfile::tempdir().unwrap();
        let mut contacts = contacts_at(&dir);
        contacts.set(ALICE, "Alice".to_string()).unwrap();
        assert_eq!(
            contacts.all(),
            HashMap::from([(ALICE, "Alice".to_string())])
        );
    }

    #[test]
    fn set_again_overwrites_the_existing_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut contacts = contacts_at(&dir);
        contacts.set(ALICE, "Alice".to_string()).unwrap();
        contacts.set(ALICE, "Alicia".to_string()).unwrap();
        assert_eq!(
            contacts.all(),
            HashMap::from([(ALICE, "Alicia".to_string())]),
            "re-aliasing the same id must rename, not duplicate"
        );
    }

    #[test]
    fn contacts_are_independent() {
        let dir = tempfile::tempdir().unwrap();
        let mut contacts = contacts_at(&dir);
        contacts.set(ALICE, "Alice".to_string()).unwrap();
        contacts.set(BOB, "Bob".to_string()).unwrap();
        assert_eq!(
            contacts.all(),
            HashMap::from([(ALICE, "Alice".to_string()), (BOB, "Bob".to_string())])
        );
    }

    #[test]
    fn persists_across_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("contacts");

        let mut contacts = Contacts::load(path.clone()).unwrap();
        contacts.set(ALICE, "Alice".to_string()).unwrap();

        let reloaded = Contacts::load(path).unwrap();
        assert_eq!(
            reloaded.all(),
            HashMap::from([(ALICE, "Alice".to_string())])
        );
    }
}
