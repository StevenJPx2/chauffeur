use std::fs::DirBuilder;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::DaemonError;

/// Where one installation keeps its socket, configuration and shipped
/// content. The name is the executable's, so `chauffeur-next` keeps its own
/// socket and `~/.config/chauffeur-next/` beside `chauffeur`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    /// The installation's name, such as `chauffeur`.
    pub name: String,
    /// The daemon's socket.
    pub socket: PathBuf,
    /// The user's configuration: `models.json`, `rules/`, `redaction.json`.
    pub config: PathBuf,
    /// Shipped content: `models/`, `rules/`, `safety/`.
    pub distribution: PathBuf,
}

impl Paths {
    /// The paths for `name`, from the environment:
    ///
    /// - socket: `$CHAUFFEUR_SOCKET`, or `<name>/daemon.sock` in
    ///   `$XDG_RUNTIME_DIR`, or in `$TMPDIR` when that is unset;
    /// - configuration: `$XDG_CONFIG_HOME/<name>`, or `~/.config/<name>`;
    /// - distribution: `distribution`, or `$CHAUFFEUR_DISTRIBUTION`, or
    ///   `share/<name>` beside the executable's folder.
    #[must_use]
    pub fn from_env(name: &str, distribution: Option<PathBuf>) -> Self {
        let var = |key: &str| {
            std::env::var_os(key)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let runtime = var("XDG_RUNTIME_DIR").unwrap_or_else(std::env::temp_dir);
        let socket =
            var("CHAUFFEUR_SOCKET").unwrap_or_else(|| runtime.join(name).join("daemon.sock"));
        let config = var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".config")))
            .unwrap_or_default()
            .join(name);
        let beside_executable = std::env::current_exe().ok().and_then(|exe| {
            exe.parent()
                .and_then(Path::parent)
                .map(|prefix| prefix.join("share").join(name))
        });
        let distribution = distribution
            .or_else(|| var("CHAUFFEUR_DISTRIBUTION"))
            .or(beside_executable)
            .unwrap_or_default();
        Self {
            name: name.to_owned(),
            socket,
            config,
            distribution,
        }
    }
}

/// Creates the socket's folder with mode `0700`, or checks that an existing
/// one has that mode and the same owner as the user's home folder, so only
/// the user's own processes can reach the socket.
///
/// # Errors
///
/// [`DaemonError::SocketFolder`] when the folder cannot be created, or is
/// open to others or owned by someone else.
pub fn private_folder(socket: &Path) -> Result<(), DaemonError> {
    let folder = socket
        .parent()
        .ok_or_else(|| refused(socket, "has no folder"))?;
    if !folder.exists() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(folder)
            .map_err(|error| refused(folder, &error.to_string()))?;
    }
    let metadata =
        std::fs::metadata(folder).map_err(|error| refused(folder, &error.to_string()))?;
    if metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(refused(folder, "must have mode 0700"));
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| refused(folder, "HOME is not set"))?;
    let owner = std::fs::metadata(&home)
        .map_err(|error| refused(&home, &error.to_string()))?
        .uid();
    if metadata.uid() != owner {
        return Err(refused(folder, "belongs to another user"));
    }
    Ok(())
}

fn refused(path: &Path, reason: &str) -> DaemonError {
    DaemonError::SocketFolder {
        path: path.to_owned(),
        reason: reason.to_owned(),
    }
}
