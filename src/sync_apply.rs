//! Safe receive-side channel-folder application planning.
//!
//! This module never scans or publishes local files.  It validates one
//! remote relative path and reserves a conflict-safe destination for runtime.

use std::fs;
use std::path::{Component, Path, PathBuf};

pub fn reserve(root: &Path, remote: &str) -> Result<PathBuf, String> {
    let relative = Path::new(remote);
    if remote.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("sync skipped unsafe remote path {remote:?}"));
    }
    let mut dir = root.to_path_buf();
    ensure_directory(&dir)?;
    let parts: Vec<_> = relative.components().collect();
    for component in &parts[..parts.len() - 1] {
        let Component::Normal(name) = component else {
            unreachable!()
        };
        reject_case_collision(&dir, name)?;
        dir.push(name);
        ensure_directory(&dir)?;
    }
    let Component::Normal(name) = parts.last().expect("non-empty checked") else {
        unreachable!()
    };
    reject_case_collision(&dir, name)?;
    let destination = dir.join(name);
    if !destination.exists() {
        return Ok(destination);
    }
    let stem = destination
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let extension = destination.extension().and_then(|n| n.to_str());
    for number in 1..=1000 {
        let name = match extension {
            Some(ext) => format!("{stem} (remote conflict {number}).{ext}"),
            None => format!("{stem} (remote conflict {number})"),
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "sync could not reserve a conflict copy under {}",
        dir.display()
    ))
}

fn ensure_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("sync refused symlink {}", path.display()))
        }
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(format!("sync refused non-directory {}", path.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => fs::create_dir(path)
            .map_err(|err| format!("sync couldn't create {}: {err}", path.display())),
        Err(err) => Err(format!("sync couldn't inspect {}: {err}", path.display())),
    }
}

fn reject_case_collision(dir: &Path, wanted: &std::ffi::OsStr) -> Result<(), String> {
    let wanted = wanted.to_string_lossy();
    for entry in
        fs::read_dir(dir).map_err(|err| format!("sync couldn't read {}: {err}", dir.display()))?
    {
        let found = entry.map_err(|err| err.to_string())?.file_name();
        if found.to_string_lossy() != wanted
            && found.to_string_lossy().eq_ignore_ascii_case(&wanted)
        {
            return Err(format!(
                "sync skipped case-colliding path component {wanted:?}"
            ));
        }
    }
    Ok(())
}

pub fn temporary_for(destination: &Path) -> PathBuf {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    destination.with_file_name(format!(
        ".{name}.leyline-sync-{:016x}.tmp",
        rand::random::<u64>()
    ))
}

pub fn finalize(temp: &Path, destination: &Path) -> Result<(), String> {
    fs::rename(temp, destination).map_err(|err| {
        format!(
            "sync couldn't atomically apply {}: {err}",
            destination.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_escape_and_preserves_local_file_as_conflict() {
        let root = tempfile::tempdir().unwrap();
        assert!(reserve(root.path(), "../outside").is_err());
        fs::write(root.path().join("note.txt"), b"local").unwrap();
        assert!(
            reserve(root.path(), "note.txt")
                .unwrap()
                .ends_with("note (remote conflict 1).txt")
        );
    }

    #[test]
    fn finalization_is_a_sibling_rename() {
        let root = tempfile::tempdir().unwrap();
        let destination = root.path().join("note.txt");
        let temporary = temporary_for(&destination);
        fs::write(&temporary, b"remote").unwrap();
        assert!(!destination.exists());
        finalize(&temporary, &destination).unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"remote");
        assert!(!temporary.exists());
    }
}
