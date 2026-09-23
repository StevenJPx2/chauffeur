//! Strict, bounded JSON config files for capabilities.

use std::path::Path;

use serde::de::DeserializeOwned;

pub const MAX_CONFIG_BYTES: u64 = 16_384;
const MAX_DIRECTORY_ENTRIES: usize = 256;

/// Load `path` as strict JSON, or `T::default()` when the file does not exist.
/// Config types should use `#[serde(deny_unknown_fields)]`.
pub fn load_config<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(error) => return Err(format!("stat {}: {error}", path.display())),
    };

    if !metadata.is_file() || metadata.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "{} must be a regular file of at most {MAX_CONFIG_BYTES} bytes",
            path.display()
        ));
    }

    let source =
        std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;

    serde_json::from_slice(&source)
        .map_err(|error| format!("{}: invalid config: {error}", path.display()))
}

/// Read every `*.json` file directly in `directory`, sorted by path. A missing
/// directory has none. At most `max_files` files of `max_bytes` each.
pub fn read_json_files(
    directory: &Path,
    max_files: usize,
    max_bytes: u64,
) -> Result<Vec<(std::path::PathBuf, Vec<u8>)>, String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("read {}: {error}", directory.display())),
    };
    let mut paths = Vec::new();

    for (index, entry) in entries.enumerate() {
        if index == MAX_DIRECTORY_ENTRIES {
            return Err(format!(
                "{} holds more than {MAX_DIRECTORY_ENTRIES} entries",
                directory.display()
            ));
        }

        let path = entry
            .map_err(|error| format!("read {}: {error}", directory.display()))?
            .path();

        if path.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            if paths.len() == max_files {
                return Err(format!(
                    "{} holds more than {max_files} JSON files",
                    directory.display()
                ));
            }

            paths.push(path);
        }
    }

    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let size = std::fs::metadata(&path)
                .map_err(|error| format!("stat {}: {error}", path.display()))?
                .len();

            if size > max_bytes {
                return Err(format!("{} exceeds {max_bytes} bytes", path.display()));
            }

            let bytes = std::fs::read(&path)
                .map_err(|error| format!("read {}: {error}", path.display()))?;

            Ok((path, bytes))
        })
        .collect()
}
