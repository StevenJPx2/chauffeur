//! Redacts evidence before any of it reaches a model. Known credential
//! formats and private-key blocks become `[REDACTED]`. Other
//! credential-like strings become a numbered description of their shape,
//! never their value. Paths, links and bare hashes stay readable.

mod config;
mod key;
mod shape;
mod walk;

use std::collections::BTreeMap;
use std::path::PathBuf;

pub use config::{Prefix, RedactError, RedactionConfig};

use config::MAX_PREFIXES;
use key::{key_before, path_like};
use shape::{describe, shape};
use walk::{is_token_char, walk};

/// What a known secret is replaced with.
pub const REDACTED: &str = "[REDACTED]";

/// Numbers each described value within one navigation, so the same value
/// gets the same label in every block.
#[derive(Debug, Default)]
pub struct Masking {
    numbers: BTreeMap<String, usize>,
}

impl Masking {
    fn number(&mut self, value: &str) -> usize {
        let next = self.numbers.len().saturating_add(1);
        *self.numbers.entry(value.to_owned()).or_insert(next)
    }
}

/// Redacts text with the configured prefixes and the built-in shape rules.
#[derive(Clone, Debug)]
pub struct Redactor {
    prefixes: Vec<Prefix>,
}

impl Redactor {
    /// A redactor from `configs` in order, shipped first.
    ///
    /// # Errors
    ///
    /// [`RedactError::Prefixes`] with more than 256 prefixes or an empty one.
    pub fn new(configs: impl IntoIterator<Item = RedactionConfig>) -> Result<Self, RedactError> {
        let mut prefixes = Vec::new();
        for config in configs {
            if config.replace {
                prefixes.clear();
            }
            prefixes.extend(config.prefixes);
        }
        if prefixes.len() > MAX_PREFIXES || prefixes.iter().any(|prefix| prefix.prefix.is_empty()) {
            return Err(RedactError::Prefixes);
        }
        // Longer prefixes first, so `sk-ant-` wins over `sk-`.
        prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.prefix.len()));
        Ok(Self { prefixes })
    }

    /// A redactor from the files at `paths`, shipped first.
    ///
    /// # Errors
    ///
    /// A file cannot be read or parsed, or its prefixes are refused.
    pub fn load(paths: &[PathBuf]) -> Result<Self, RedactError> {
        let configs = paths
            .iter()
            .map(|path| {
                let text = std::fs::read_to_string(path).map_err(|source| RedactError::Read {
                    path: path.clone(),
                    source,
                })?;
                serde_json::from_str(&text).map_err(|source| RedactError::Parse {
                    path: path.clone(),
                    source,
                })
            })
            .collect::<Result<Vec<RedactionConfig>, _>>()?;
        Self::new(configs)
    }

    /// `text` as it may be sent.
    #[must_use]
    pub fn redact(&self, text: &str, masking: &mut Masking) -> String {
        walk(text, |word, before| {
            if path_like(word, before) {
                return word
                    .split('/')
                    .map(|segment| self.replace_word(segment, before, masking))
                    .collect::<Vec<_>>()
                    .join("/");
            }
            self.replace_word(word, before, masking)
        })
    }

    /// A word as sent: redacted, described, or unchanged.
    fn replace_word(&self, word: &str, before: &str, masking: &mut Masking) -> String {
        if self.known(word) {
            return REDACTED.to_owned();
        }
        let Some(shape) = shape(word) else {
            return word.to_owned();
        };
        let key = key_before(before);
        // A bare hex string is a commit or content hash; after a key it may be a credential.
        if shape.is_hex() && key.is_empty() {
            return word.to_owned();
        }
        describe(masking.number(word), &shape, key)
    }

    fn known(&self, word: &str) -> bool {
        self.prefixes.iter().any(|prefix| {
            word.strip_prefix(prefix.prefix.as_str())
                .is_some_and(|body| {
                    body.len() >= prefix.min_body && body.chars().all(is_token_char)
                })
        })
    }
}
