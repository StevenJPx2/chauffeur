use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The four tiers a step's judge can name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// JavaScript facts, with no model.
    Facts,
    /// A fast classifier.
    SystemOne,
    /// A reasoning model.
    SystemTwo,
    /// The user.
    Person,
}

impl Tier {
    /// The tier named `name`, such as `system-one`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Facts, Self::SystemOne, Self::SystemTwo, Self::Person]
            .into_iter()
            .find(|tier| tier.name() == name)
    }

    /// The tier's name in rule files and `models.json`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Facts => "facts",
            Self::SystemOne => "system-one",
            Self::SystemTwo => "system-two",
            Self::Person => "person",
        }
    }
}

/// Where a model's key comes from. Only `crates/judges` reads it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeySource {
    /// The environment variable holding the key.
    pub env: String,
}

/// A model that speaks the `/v1/systemone` API.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemOneEntry {
    /// The API's base URL; requests go to `{endpoint}/v1/systemone`.
    pub endpoint: String,
    /// The model ID sent with each request.
    pub model: String,
    /// The key, sent as a bearer token.
    pub key: KeySource,
    /// How long one call may take, in milliseconds.
    pub timeout_ms: Option<u64>,
}

/// Recorded or fixed answers that stand in for a real model.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaceholderEntry {
    /// A file of answers matched by evidence, relative to the `models.json`
    /// that names it.
    pub answers: Option<PathBuf>,
    /// The probabilities given for any question no recorded answer matches.
    pub otherwise: Option<BTreeMap<String, f64>>,
}

/// A registered model: its API format and that format's settings.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "format", rename_all = "lowercase")]
pub enum ModelEntry {
    /// The `/v1/systemone` API that Jev speaks.
    Systemone(SystemOneEntry),
    /// Recorded or fixed answers.
    Placeholder(PlaceholderEntry),
}

/// Why model configuration was refused.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// A file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// A file is not valid configuration.
    #[error("{path} is not valid: {source}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// Why.
        source: serde_json::Error,
    },
    /// A placeholder's recorded answer is malformed.
    #[error("answer {index} in {path} {reason}")]
    Placeholder {
        /// The answers file.
        path: PathBuf,
        /// The answer's position, from 0.
        index: usize,
        /// What is wrong with it.
        reason: String,
    },
    /// A tier names a model no file registers.
    #[error("the {tier} tier names `{model}`, which no models.json registers")]
    UnknownModel {
        /// The tier's name.
        tier: &'static str,
        /// The model it names.
        model: String,
    },
}

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

impl ModelEntry {
    fn relative_to(self, folder: &Path) -> Self {
        match self {
            Self::Placeholder(PlaceholderEntry { answers, otherwise }) => {
                Self::Placeholder(PlaceholderEntry {
                    answers: answers.map(|answers| folder.join(answers)),
                    otherwise,
                })
            }
            other @ Self::Systemone(_) => other,
        }
    }
}
