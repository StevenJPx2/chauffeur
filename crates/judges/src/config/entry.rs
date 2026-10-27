use std::path::Path;

use serde::Deserialize;

use crate::placeholder::PlaceholderEntry;
use crate::tiers::system_one::SystemOneEntry;

/// Where a model's key comes from. Only `crates/judges` reads it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeySource {
    /// The environment variable holding the key.
    pub env: String,
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

impl ModelEntry {
    /// The entry with its file paths made relative to `folder`, the folder of
    /// the `models.json` that names it.
    pub(super) fn relative_to(self, folder: &Path) -> Self {
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
