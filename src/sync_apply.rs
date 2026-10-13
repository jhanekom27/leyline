//! Safe receive-side channel-folder application planning.
//!
//! This module validates remote paths and performs filesystem moves requested
//! by the runtime. It never scans or publishes local files.

use std::fs;
use std::path::{Component, Path, PathBuf};

const TRASH_DIR: &str = ".leyline-trash";

#[derive(Debug, PartialEq, Eq)]
pub enum Relocation {
    Moved(PathBuf),
    AlreadyThere,
    Unavailable,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Trash {
    Moved(PathBuf),
    Missing,
}

pub fn reserve(root: &Path, remote: &str) -> Result<PathBuf, String> {
    let destination = exact_destination(root, remote)?;
    if !destination.exists() {
        return Ok(destination);
    }
    let dir = destination.parent().expect("destination has parent");
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

/// Moves a verified remote copy into a newly announced path without
/// downloading its bytes again. `source` may be in prior trash, allowing a
/// delete and rename to arrive in either order.
pub fn relocate(root: &Path, source: &Path, remote: &str) -> Result<Relocation, String> {
    let destination = exact_destination(root, remote)?;
    if source == destination {
        return Ok(Relocation::AlreadyThere);
    }
    if !source.starts_with(root) {
        return Err(format!(
            "sync refused rename source outside channel folder {}",
            source.display()
        ));
    }
    match fs::symlink_metadata(source) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("sync refused symlink {}", source.display()));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!("sync refused non-file {}", source.display()));
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Relocation::Unavailable);
        }
        Err(err) => return Err(format!("sync couldn't inspect {}: {err}", source.display())),
    }
    if destination.exists() {
        return Ok(Relocation::Unavailable);
    }
    fs::rename(source, &destination).map_err(|err| {
        format!(
            "sync couldn't rename {} to {}: {err}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(Relocation::Moved(destination))
}

/// Moves a validated remote path aside rather than deleting it. The hidden
/// trash folder is excluded by the local sync scanner.
pub fn trash(root: &Path, remote: &str) -> Result<Trash, String> {
    let source = exact_destination(root, remote)?;
    match fs::symlink_metadata(&source) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("sync refused symlink {}", source.display()));
        }
        Ok(metadata) if !metadata.is_file() => {
            return Err(format!("sync refused non-file {}", source.display()));
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Trash::Missing),
        Err(err) => return Err(format!("sync couldn't inspect {}: {err}", source.display())),
    }
    let target = trash_destination(root, remote)?;
    fs::rename(&source, &target).map_err(|err| {
        format!(
            "sync couldn't move {} to trash {}: {err}",
            source.display(),
            target.display()
        )
    })?;
    Ok(Trash::Moved(target))
}

fn exact_destination(root: &Path, remote: &str) -> Result<PathBuf, String> {
    let parts = validated_parts(remote)?;
    let mut dir = root.to_path_buf();
    ensure_directory(&dir)?;
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
    Ok(dir.join(name))
}

fn trash_destination(root: &Path, remote: &str) -> Result<PathBuf, String> {
    let parts = validated_parts(remote)?;
    let mut dir = root.join(TRASH_DIR);
    ensure_directory(&dir)?;
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
    let candidate = dir.join(name);
    if !candidate.exists() {
        return Ok(candidate);
    }
    let stem = candidate
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let extension = candidate.extension().and_then(|name| name.to_str());
    for number in 1..=1000 {
        let name = match extension {
            Some(ext) => format!("{stem} (trash conflict {number}).{ext}"),
            None => format!("{stem} (trash conflict {number})"),
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "sync could not reserve trash under {}",
        dir.display()
    ))
}

fn validated_parts(remote: &str) -> Result<Vec<Component<'_>>, String> {
    let relative = Path::new(remote);
    if remote.is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("sync skipped unsafe remote path {remote:?}"));
    }
    Ok(relative.components().collect())
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

    #[test]
    fn remote_delete_moves_only_a_validated_regular_file_to_hidden_trash() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("nested/report.txt");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"remote").unwrap();

        let target = match trash(root.path(), "nested/report.txt").unwrap() {
            Trash::Moved(path) => path,
            Trash::Missing => panic!("existing file must be moved"),
        };

        assert_eq!(target, root.path().join(".leyline-trash/nested/report.txt"));
        assert_eq!(fs::read(target).unwrap(), b"remote");
        assert!(!source.exists());
        assert!(trash(root.path(), "../outside").is_err());
    }

    #[test]
    fn relocation_reuses_matching_content_without_overwriting_a_conflict() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join(".leyline-trash/old.txt");
        fs::create_dir_all(source.parent().unwrap()).unwrap();
        fs::write(&source, b"same bytes").unwrap();

        assert_eq!(
            relocate(root.path(), &source, "new.txt").unwrap(),
            Relocation::Moved(root.path().join("new.txt"))
        );
        assert_eq!(
            fs::read(root.path().join("new.txt")).unwrap(),
            b"same bytes"
        );
    }
}
