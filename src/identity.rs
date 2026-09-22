//! Local identity: a persisted `iroh::SecretKey` that *is* the user's
//! identity -- no accounts, no server. Loads the key from disk on startup,
//! generating and persisting a new one on first run -- see concept.md's
//! "Identity & channels" section.

use std::io::ErrorKind;
use std::path::Path;

use anyhow::Context;
use iroh::SecretKey;

/// Loads the secret key persisted at `path`, generating and persisting a new
/// one if the file doesn't exist yet.
///
/// A wrong-length file or any other I/O error fails clearly rather than
/// silently falling back to a fresh (and thus different) identity.
pub fn load_or_generate(path: &Path) -> anyhow::Result<SecretKey> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let bytes: [u8; 32] = bytes.try_into().map_err(|bytes: Vec<u8>| {
                anyhow::anyhow!(
                    "identity file at {} has {} bytes, expected 32",
                    path.display(),
                    bytes.len()
                )
            })?;
            Ok(SecretKey::from_bytes(&bytes))
        }
        Err(err) if err.kind() == ErrorKind::NotFound => generate_and_persist(path),
        Err(err) => {
            Err(err).with_context(|| format!("failed to read identity from {}", path.display()))
        }
    }
}

fn generate_and_persist(path: &Path) -> anyhow::Result<SecretKey> {
    let key = SecretKey::generate();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    std::fs::write(path, key.to_bytes())
        .with_context(|| format!("failed to write identity to {}", path.display()))?;
    restrict_permissions(path)?;
    Ok(key)
}

/// Restricts the identity file to owner-only read/write, since it's a secret
/// key. A no-op on non-unix targets, which have no equivalent bit to set
/// here.
#[cfg(unix)]
fn restrict_permissions(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("failed to set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_and_persists_on_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");

        assert!(!path.exists());
        let key = load_or_generate(&path).unwrap();
        assert!(path.exists());

        let reloaded = load_or_generate(&path).unwrap();
        assert_eq!(key.to_bytes(), reloaded.to_bytes());
    }

    #[test]
    fn rejects_a_wrong_length_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");
        std::fs::write(&path, b"too short").unwrap();

        let err = load_or_generate(&path).unwrap_err();
        assert!(err.to_string().contains("32"));
    }

    #[cfg(unix)]
    #[test]
    fn persisted_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity");
        load_or_generate(&path).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
