mod entry;
mod error;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use entry::{KeySource, ModelEntry};
pub use error::ConfigError;
use serde::Deserialize;

use crate::tiers::Tier;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelsFile {
    #[serde(default)]
    models: BTreeMap<String, ModelEntry>,
    #[serde(default)]
    tiers: BTreeMap<Tier, Vec<String>>,
}

/// Every registered model, and each tier's models in fallback order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Models {
    models: BTreeMap<String, ModelEntry>,
    tiers: BTreeMap<Tier, Vec<String>>,
}

impl Models {
    /// Loads `files` in order, shipped first and the user's last: a later
    /// file's model replaces an earlier one of the same name, and a later
    /// file's tier list replaces the earlier list. A placeholder's `answers`
    /// path becomes relative to the file that names it.
    ///
    /// # Errors
    ///
    /// The first file that cannot be read or parsed, or a tier naming a
    /// model no file registers.
    pub fn load(files: &[PathBuf]) -> Result<Self, ConfigError> {
        let mut loaded = Self::default();
        for path in files {
            let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
                path: path.clone(),
                source,
            })?;
            let file: ModelsFile =
                serde_json::from_str(&text).map_err(|source| ConfigError::Parse {
                    path: path.clone(),
                    source,
                })?;
            let folder = path.parent().unwrap_or_else(|| Path::new(""));
            for (name, entry) in file.models {
                loaded.models.insert(name, entry.relative_to(folder));
            }
            loaded.tiers.extend(file.tiers);
        }
        loaded.check()?;
        Ok(loaded)
    }

    fn check(&self) -> Result<(), ConfigError> {
        for (tier, names) in &self.tiers {
            if let Some(model) = names.iter().find(|name| !self.models.contains_key(*name)) {
                return Err(ConfigError::UnknownModel {
                    tier: tier.name(),
                    model: model.clone(),
                });
            }
        }
        Ok(())
    }

    /// The model registered as `name`.
    #[must_use]
    pub fn model(&self, name: &str) -> Option<&ModelEntry> {
        self.models.get(name)
    }

    /// The tier's model names, in fallback order.
    #[must_use]
    pub fn tier(&self, tier: Tier) -> &[String] {
        self.tiers.get(&tier).map_or(&[], Vec::as_slice)
    }
}
