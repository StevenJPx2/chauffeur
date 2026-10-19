//! Configured source ownership: cheap change checks, descriptions, and
//! per-skill files fetched only when a skill is attached. The daemon owns
//! scheduling; no judge or host adapter does source I/O.

mod approve;
mod git;
mod http;
mod model;
mod parse;
mod review;
mod skilld;
mod store;

#[cfg(test)]
mod discovery_tests;
#[cfg(test)]
mod tests;

pub use approve::{Approval, Change, Consent, approve, read_config, write_config};
pub use model::{Source, SourcesConfig};
pub use review::{Gathered, Review, gather, read_verdict, review_signal};
pub use skilld::{
    Behavior, Candidate, CandidateFile, DiscoveryError, Found, Selector, SkilldClient, Tier,
};
pub use store::SourceStore;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The largest `skill-sources.json` the daemon reads or approval writes.
pub(super) const MAX_CONFIG: usize = 16_384;
pub(super) const MAX_FILE: usize = 1024 * 1024;
pub(super) const MAX_FILES: usize = 512;
pub(super) const MAX_BYTES: usize = 16 * 1024 * 1024;
/// Stored revisions per source; old ones are never evicted beneath a session.
pub(super) const MAX_REVISIONS: usize = 32;
/// Files named in one skill's text; the folder itself always has them all.
pub(super) const MAX_LISTED_FILES: usize = 100;

#[derive(Debug, Error)]
pub enum SourceError {
    #[error("source I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("source JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("skill YAML: {0}")]
    Yaml(#[from] yaml_serde::Error),
    #[error("source HTTP: {0}")]
    Http(#[from] reqwest::Error),
    #[error("{0}")]
    Invalid(&'static str),
    #[error("Git command failed: {0}")]
    Git(std::process::ExitStatus),
    #[error("source work exceeded its deadline")]
    Timeout,
}

/// What a source offers at one revision: descriptions and bodies, plus the
/// files each skill has upstream. Files are fetched only on attach.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Listing {
    pub source: Source,
    pub revision: String,
    pub checked_at: u64,
    /// HTTP cache validators for the catalog index.
    #[serde(default)]
    pub validator: Validator,
    pub skills: Vec<ListedSkill>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Validator {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ListedSkill {
    pub id: String,
    pub description: String,
    pub body: String,
    /// The skill's folder upstream, relative to the source root.
    pub dir: String,
    pub files: Vec<ListedFile>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct ListedFile {
    /// Relative to the skill's folder.
    pub path: String,
    pub executable: bool,
    /// The Git blob, fetched lazily from the partial clone; `None` for HTTP.
    pub object: Option<String>,
}

/// A change check's outcome.
pub(super) enum Check {
    Unchanged,
    Changed(Listing),
}

pub(super) fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|part| {
            !part.is_empty()
                && !matches!(part, "." | ".." | ".git" | ".repository")
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_. -".contains(&b))
        })
}
