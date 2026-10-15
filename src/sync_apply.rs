//! Safe receive-side channel-folder application planning.
//!
//! This module validates remote paths and performs filesystem moves requested
//! by the runtime. It never scans or publishes local files.

use std::fs;
use std::path::{Component, Path, PathBuf};

const TRASH_DIR: &str = ".leyline-trash";
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedFile {
    pub relative: String,
    pub size: u64,
}

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
#[derive(Debug, PartialEq, Eq)]
pub enum Restore {
    Restored(PathBuf),
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

/// Lists recoverable remote deletes below a channel's private trash folder.
pub fn list_trash(root: &Path) -> Result<Vec<TrashedFile>, String> {
    let trash = trash_root(root)?;
    match fs::symlink_metadata(&trash) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("sync refused symlink {}", trash.display()));
        }
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(format!("sync refused non-directory {}", trash.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(format!("sync couldn't inspect {}: {err}", trash.display())),
    }
    let mut files = Vec::new();
    list_trash_directory(&trash, &trash, &mut files)?;
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok(files)
}

/// Restores a validated path without overwriting any local file.
pub fn restore(root: &Path, relative: &str) -> Result<Restore, String> {
    let parts = validated_parts(relative)?;
    let trash = trash_root(root)?;
    let source = match trash_source(&trash, &parts)? {
        Some(source) => source,
        None => return Ok(Restore::Missing),
    };
    let destination = restored_destination(root, relative)?;
    fs::rename(&source, &destination).map_err(|err| {
        format!(
            "sync couldn't restore {} to {}: {err}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(Restore::Restored(destination))
}

/// Permanently removes recoverable files only after the caller explicitly
/// requests a purge. There is intentionally no automatic expiry.
pub fn purge_trash(root: &Path) -> Result<usize, String> {
    let files = list_trash(root)?;
    if files.is_empty() {
        return Ok(0);
    }
    let trash = trash_root(root)?;
    fs::remove_dir_all(&trash)
        .map_err(|err| format!("sync couldn't purge {}: {err}", trash.display()))?;
    Ok(files.len())
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

fn trash_root(root: &Path) -> Result<PathBuf, String> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(format!("sync refused symlink {}", root.display()))
        }
        Ok(metadata) if metadata.is_dir() => Ok(root.join(TRASH_DIR)),
        Ok(_) => Err(format!("sync refused non-directory {}", root.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(root.join(TRASH_DIR)),
        Err(err) => Err(format!("sync couldn't inspect {}: {err}", root.display())),
    }
}

fn list_trash_directory(
    trash: &Path,
    directory: &Path,
    files: &mut Vec<TrashedFile>,
) -> Result<(), String> {
    for entry in fs::read_dir(directory)
        .map_err(|err| format!("sync couldn't read {}: {err}", directory.display()))?
    {
        let entry =
            entry.map_err(|err| format!("sync couldn't read {}: {err}", directory.display()))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .map_err(|err| format!("sync couldn't inspect {}: {err}", path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!("sync refused symlink {}", path.display()));
        }
        if metadata.is_dir() {
            list_trash_directory(trash, &path, files)?;
            continue;
        }
        if !metadata.is_file() {
            return Err(format!("sync refused non-file {}", path.display()));
        }
        let relative = path
            .strip_prefix(trash)
            .expect("recursive trash path must remain below trash root")
            .to_str()
            .ok_or_else(|| format!("sync skipped non-UTF-8 trash path {}", path.display()))?
            .replace(std::path::MAIN_SEPARATOR, "/");
        validated_parts(&relative)?;
        files.push(TrashedFile {
            relative,
            size: metadata.len(),
        });
    }
    Ok(())
}

fn trash_source(trash: &Path, parts: &[Component<'_>]) -> Result<Option<PathBuf>, String> {
    match fs::symlink_metadata(trash) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!("sync refused symlink {}", trash.display()));
        }
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Err(format!("sync refused non-directory {}", trash.display())),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("sync couldn't inspect {}: {err}", trash.display())),
    }
    let mut path = trash.to_path_buf();
    for (index, component) in parts.iter().enumerate() {
        let Component::Normal(name) = component else {
            unreachable!();
        };
        path.push(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("sync refused symlink {}", path.display()));
            }
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => {
                return Err(format!("sync couldn't inspect {}: {err}", path.display()));
            }
        };
        if index + 1 == parts.len() {
            if !metadata.is_file() {
                return Err(format!("sync refused non-file {}", path.display()));
            }
        } else if !metadata.is_dir() {
            return Err(format!("sync refused non-directory {}", path.display()));
        }
    }
    Ok(Some(path))
}

fn restored_destination(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let destination = exact_destination(root, relative)?;
    if !destination.exists() {
        return Ok(destination);
    }
    let dir = destination.parent().expect("destination has parent");
    let stem = destination
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let extension = destination.extension().and_then(|name| name.to_str());
    for number in 1..=1000 {
        let name = match extension {
            Some(ext) => format!("{stem} (restored {number}).{ext}"),
            None => format!("{stem} (restored {number})"),
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "sync could not reserve a restored copy under {}",
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

    #[test]
    fn trash_lists_nested_files_without_touching_them() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join(".leyline-trash/old/report.txt");
        let second = root.path().join(".leyline-trash/notes.txt");
        fs::create_dir_all(first.parent().unwrap()).unwrap();
        fs::write(&first, b"report").unwrap();
        fs::write(&second, b"notes").unwrap();

        assert_eq!(
            list_trash(root.path()).unwrap(),
            vec![
                TrashedFile {
                    relative: "notes.txt".into(),
                    size: 5,
                },
                TrashedFile {
                    relative: "old/report.txt".into(),
                    size: 6,
                },
            ]
        );
        assert!(first.exists());
        assert!(second.exists());
    }

    #[test]
    fn restore_keeps_a_local_file_and_uses_a_distinct_copy() {
        let root = tempfile::tempdir().unwrap();
        let trashed = root.path().join(".leyline-trash/nested/report.txt");
        let local = root.path().join("nested/report.txt");
        fs::create_dir_all(trashed.parent().unwrap()).unwrap();
        fs::create_dir_all(local.parent().unwrap()).unwrap();
        fs::write(&trashed, b"remote").unwrap();
        fs::write(&local, b"local").unwrap();

        let restored = restore(root.path(), "nested/report.txt").unwrap();

        assert_eq!(
            restored,
            Restore::Restored(root.path().join("nested/report (restored 1).txt"))
        );
        assert_eq!(fs::read(local).unwrap(), b"local");
        assert_eq!(
            fs::read(root.path().join("nested/report (restored 1).txt")).unwrap(),
            b"remote"
        );
        assert!(!trashed.exists());
    }

    #[test]
    fn purge_only_removes_trash_after_an_explicit_request() {
        let root = tempfile::tempdir().unwrap();
        let trashed = root.path().join(".leyline-trash/report.txt");
        fs::create_dir_all(trashed.parent().unwrap()).unwrap();
        fs::write(&trashed, b"report").unwrap();

        assert_eq!(list_trash(root.path()).unwrap().len(), 1);
        assert!(trashed.exists(), "listing never removes recoverable files");
        assert_eq!(purge_trash(root.path()).unwrap(), 1);
        assert!(!trashed.exists());
    }
}
