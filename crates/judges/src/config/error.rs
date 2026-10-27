use std::path::PathBuf;

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
