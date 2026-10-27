use std::path::PathBuf;

use serde::Deserialize;

/// The most prefixes redaction accepts.
pub(super) const MAX_PREFIXES: usize = 256;

/// A known credential format: a fixed prefix and a minimum body.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prefix {
    /// The prefix, such as `ghp_`.
    pub prefix: String,
    /// The fewest token characters after it.
    pub min_body: usize,
}

/// A redaction file: prefixes that add to the earlier files', or replace
/// them with `replace`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionConfig {
    /// Whether these prefixes replace the earlier files' instead of adding to them.
    #[serde(default)]
    pub replace: bool,
    /// The known credential formats.
    #[serde(default)]
    pub prefixes: Vec<Prefix>,
}

/// Why redaction could not be set up. Evidence is never sent without it.
#[derive(Debug, thiserror::Error)]
pub enum RedactError {
    /// A file could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// A file is not valid redaction configuration.
    #[error("{path} is not valid: {source}")]
    Parse {
        /// The file.
        path: PathBuf,
        /// Why.
        source: serde_json::Error,
    },
    /// Too many prefixes, or an empty one.
    #[error("redaction allows at most {MAX_PREFIXES} prefixes, none empty")]
    Prefixes,
}
