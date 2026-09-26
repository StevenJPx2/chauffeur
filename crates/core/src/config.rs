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

/// Shipped defaults overlaid by your file at `path`, when it exists: your
/// fields replace the shipped ones, objects merge field by field, and any
/// other value, lists included, replaces the shipped value whole. `T` should
/// use `#[serde(deny_unknown_fields)]`, so a misspelled field is an error.
///
/// # Errors
///
/// When the shipped defaults or your file are not valid JSON, your file is
/// not a regular file within [`MAX_CONFIG_BYTES`], or the merged value does
/// not fit `T`.
pub fn load_layered<T: DeserializeOwned>(shipped: &str, path: &Path) -> Result<T, String> {
    let mut merged: serde_json::Value = serde_json::from_str(shipped)
        .map_err(|error| format!("shipped defaults for {}: {error}", path.display()))?;
    let yours: Option<serde_json::Value> = load_config(path)?;

    if let Some(yours) = yours {
        overlay(&mut merged, yours);
    }

    serde_json::from_value(merged)
        .map_err(|error| format!("{}: invalid config: {error}", path.display()))
}

/// Merge `yours` into `base`: objects field by field, anything else whole.
fn overlay(base: &mut serde_json::Value, yours: serde_json::Value) {
    match (base, yours) {
        (serde_json::Value::Object(base), serde_json::Value::Object(yours)) => {
            for (key, value) in yours {
                match base.get_mut(&key) {
                    Some(existing) => overlay(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (base, yours) => *base = yours,
    }
}

/// Read every `*.json` file directly in `directory`, sorted by path. A missing
/// directory has none. At most `max_files` files of `max_bytes` each. A
/// symlinked file is rejected, so a contract cannot be read from outside its
/// folder.
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
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| format!("stat {}: {error}", path.display()))?;

            if !metadata.is_file() {
                return Err(format!("{} must be a regular file", path.display()));
            }

            let size = metadata.len();

            if size > max_bytes {
                return Err(format!("{} exceeds {max_bytes} bytes", path.display()));
            }

            let bytes = std::fs::read(&path)
                .map_err(|error| format!("read {}: {error}", path.display()))?;

            Ok((path, bytes))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judge::Threshold;

    #[derive(Debug, serde::Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Settings {
        bar: Threshold,
        tools: Vec<String>,
        budget: Budget,
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct Budget {
        bytes: u64,
        count: usize,
    }

    const SHIPPED: &str = r#"{
      "bar": { "at": 0.7, "confidence": 0.4 },
      "tools": ["read", "edit"],
      "budget": { "bytes": 65536, "count": 4 }
    }"#;

    fn yours(name: &str, json: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "chauffeur-layered-{}-{name}.json",
            std::process::id()
        ));

        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn a_missing_file_leaves_the_shipped_defaults() {
        let settings: Settings =
            load_layered(SHIPPED, Path::new("/nonexistent/settings.json")).unwrap();

        assert_eq!(
            settings.budget,
            Budget {
                bytes: 65_536,
                count: 4
            }
        );
        assert_eq!(settings.tools, ["read", "edit"]);
    }

    #[test]
    fn your_fields_merge_objects_and_replace_lists() {
        let path = yours(
            "merge",
            r#"{ "budget": { "count": 2 }, "tools": ["read"] }"#,
        );
        let settings: Settings = load_layered(SHIPPED, &path).unwrap();

        assert_eq!(
            settings.budget,
            Budget {
                bytes: 65_536,
                count: 2
            }
        );
        assert_eq!(settings.tools, ["read"]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_misspelled_field_or_an_out_of_range_bar_is_an_error() {
        let typo = yours("typo", r#"{ "budjet": { "count": 2 } }"#);
        let range = yours("range", r#"{ "bar": { "at": 1.5 } }"#);

        assert!(
            load_layered::<Settings>(SHIPPED, &typo)
                .unwrap_err()
                .contains("budjet")
        );
        assert!(
            load_layered::<Settings>(SHIPPED, &range)
                .unwrap_err()
                .contains("outside [0, 1]")
        );
        std::fs::remove_file(typo).unwrap();
        std::fs::remove_file(range).unwrap();
    }
}
