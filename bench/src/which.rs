//! Resolve programs against the harness's own `PATH`, before task stubs are
//! prepended for children.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// An executable path for `program`: taken as given when it contains a `/`,
/// else the first executable match on `PATH`.
///
/// # Errors
/// No such executable.
pub fn resolve(program: &str) -> Result<PathBuf, String> {
    if program.contains('/') {
        let path = PathBuf::from(program);

        return if executable(&path) {
            path.canonicalize()
                .map_err(|error| format!("resolve {program}: {error}"))
        } else {
            Err(format!("{program} is not an executable file"))
        };
    }

    let paths = std::env::var_os("PATH").unwrap_or_default();

    std::env::split_paths(&paths)
        .map(|dir| dir.join(program))
        .find(|candidate| executable(candidate))
        .ok_or_else(|| format!("{program} was not found on PATH"))
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}
