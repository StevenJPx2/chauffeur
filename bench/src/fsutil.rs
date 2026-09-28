//! Directory copies used to lay out a run's repository.

use std::path::Path;

/// Deeper trees than this are refused rather than followed.
const MAX_DEPTH: usize = 64;

/// Copy `src` into `dst`, overwriting files that exist in both. Symlinks are
/// recreated, not followed; permissions are kept. Top-level entries named in
/// `skip` are left out.
///
/// # Errors
/// Any unreadable source, unwritable destination, or a tree deeper than
/// [`MAX_DEPTH`].
pub fn overlay(src: &Path, dst: &Path, skip: &[&str]) -> Result<(), String> {
    copy_level(src, dst, skip, 0)
}

fn copy_level(src: &Path, dst: &Path, skip: &[&str], depth: usize) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{} is nested too deeply", src.display()));
    }

    std::fs::create_dir_all(dst).map_err(|error| format!("create {}: {error}", dst.display()))?;
    let entries =
        std::fs::read_dir(src).map_err(|error| format!("read {}: {error}", src.display()))?;

    for entry in entries {
        let entry = entry.map_err(|error| format!("read {}: {error}", src.display()))?;
        let name = entry.file_name();
        if depth == 0 && skip.iter().any(|skipped| name == *skipped) {
            continue;
        }

        let from = entry.path();
        let to = dst.join(&name);
        let kind = entry
            .file_type()
            .map_err(|error| format!("stat {}: {error}", from.display()))?;

        if kind.is_symlink() {
            copy_symlink(&from, &to)?;
        } else if kind.is_dir() {
            copy_level(&from, &to, skip, depth.saturating_add(1))?;
        } else {
            std::fs::copy(&from, &to)
                .map_err(|error| format!("copy {} to {}: {error}", from.display(), to.display()))?;
        }
    }

    Ok(())
}

fn copy_symlink(from: &Path, to: &Path) -> Result<(), String> {
    let target =
        std::fs::read_link(from).map_err(|error| format!("read {}: {error}", from.display()))?;

    if std::fs::symlink_metadata(to).is_ok() {
        std::fs::remove_file(to).map_err(|error| format!("replace {}: {error}", to.display()))?;
    }

    std::os::unix::fs::symlink(&target, to)
        .map_err(|error| format!("link {}: {error}", to.display()))
}

/// Remove `path` if it exists.
///
/// # Errors
/// A path that exists but cannot be removed.
pub fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("remove {}: {error}", path.display())),
    }
}
