//! The router's tunable defaults, shipped in `skills/config/model-router.json`
//! and overlaid by your `model-router.json`.

use std::path::Path;

use chauffeur_core::{Confidence, Threshold, load_layered};
use serde::Deserialize;

/// The shipped defaults, compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/model-router.json");
const MAX_PINS: usize = 64;
/// The most options a choice may offer System One, whatever you configure.
pub const MAX_CANDIDATES: usize = 32;
/// Phrases or types one word list may hold.
const MAX_WORDS: usize = 256;

/// Strict JSON config. `pins` orders preferred fallbacks as `provider/model`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelRouterConfig {
    pub pins: Vec<String>,
    /// Below this, a failover choice is treated as unavailable and the
    /// posture applies.
    pub pick_confidence: Confidence,
    pub switch_back: SwitchBack,
    /// Options offered to System One, after ordering; keeps the choice small.
    pub max_candidates: usize,
    /// A usage limit that another model avoids.
    pub limit: ErrorWords,
    /// A model that cannot serve the agent at all (no access, unknown model),
    /// as opposed to a limit that may clear.
    pub unusable: ErrorWords,
}

/// When to judge returning to the model an agent left.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SwitchBack {
    /// Judged no sooner than this after leaving a model.
    pub after_seconds: u64,
    /// Switch back on a confident yes at this bar.
    pub bar: Threshold,
}

/// Words that classify a model error, matched case-insensitively.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ErrorWords {
    /// Phrases found anywhere in the error message.
    pub messages: Vec<String>,
    /// Exact error types, spaces and dashes read as underscores.
    pub types: Vec<String>,
}

impl ModelRouterConfig {
    /// The shipped defaults overlaid by your `model-router.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, or out of bounds.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered::<Self>(SHIPPED, path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The config, if within bounds, with its word lists normalized.
    ///
    /// # Errors
    ///
    /// Names the first field out of bounds.
    pub fn checked(mut self) -> Result<Self, String> {
        if self.pins.len() > MAX_PINS {
            return Err(format!("more than {MAX_PINS} pins"));
        }

        if !(1..=MAX_CANDIDATES).contains(&self.max_candidates) {
            return Err(format!("max_candidates must be 1 to {MAX_CANDIDATES}"));
        }

        self.limit = self.limit.checked("limit")?;
        self.unusable = self.unusable.checked("unusable")?;

        Ok(self)
    }

    /// Whether the error is a usage limit another model avoids.
    #[must_use]
    pub fn is_limit_error(&self, error_type: &str, status: Option<u16>, message: &str) -> bool {
        // 503 is a provider out of capacity ("high demand"), which another model avoids.
        // Some providers report an exhausted balance as a plain invalid request,
        // which the message phrases catch.
        matches!(status, Some(402 | 429 | 503 | 529)) || self.limit.matches(error_type, message)
    }

    /// Whether the error means the model cannot serve the agent at all.
    #[must_use]
    pub fn is_unusable_error(&self, error_type: &str, status: Option<u16>, message: &str) -> bool {
        matches!(status, Some(401 | 403 | 404)) || self.unusable.matches(error_type, message)
    }
}

impl Default for ModelRouterConfig {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
            .expect("shipped model-router defaults are valid")
    }
}

impl ErrorWords {
    fn checked(self, name: &str) -> Result<Self, String> {
        if self.messages.len() > MAX_WORDS || self.types.len() > MAX_WORDS {
            return Err(format!("{name}: more than {MAX_WORDS} words in a list"));
        }

        if self
            .messages
            .iter()
            .chain(&self.types)
            .any(|word| word.trim().is_empty())
        {
            return Err(format!("{name}: an empty word"));
        }

        Ok(Self {
            messages: self.messages.iter().map(|m| m.to_lowercase()).collect(),
            types: self.types.iter().map(|kind| normalized(kind)).collect(),
        })
    }

    fn matches(&self, error_type: &str, message: &str) -> bool {
        let normalized = normalized(error_type);
        // OpenCode namespaces provider errors, e.g. `provider.quota`.
        let kind = normalized.strip_prefix("provider.").unwrap_or(&normalized);
        let message = message.to_lowercase();

        self.types.iter().any(|known| known == kind)
            || self.messages.iter().any(|phrase| message.contains(phrase))
    }
}

fn normalized(error_type: &str) -> String {
    error_type.trim().to_lowercase().replace([' ', '-'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yours(name: &str, json: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "chauffeur-model-router-{}-{name}.json",
            std::process::id()
        ));

        std::fs::write(&path, json).unwrap();
        path
    }

    #[test]
    fn the_shipped_file_holds_the_previous_defaults() {
        let config = ModelRouterConfig::load(Path::new("/nonexistent/model-router.json")).unwrap();

        assert_eq!(config, ModelRouterConfig::default());
        assert!(config.pins.is_empty());
        assert!((config.pick_confidence.value() - 0.2).abs() < f32::EPSILON);
        assert_eq!(config.switch_back.after_seconds, 300);
        assert_eq!(config.max_candidates, 8);
        assert_eq!(config.limit.messages.len(), 10);
        assert_eq!(config.limit.types.len(), 11);
        assert_eq!(config.unusable.messages.len(), 6);
        assert_eq!(config.unusable.types.len(), 10);
        assert_eq!(config.limit.types[0], "capacity_exhausted");
        assert_eq!(config.unusable.messages[0], "api key");
    }

    #[test]
    fn your_nested_field_keeps_the_rest() {
        let path = yours("nested", r#"{ "switch_back": { "after_seconds": 60 } }"#);
        let config = ModelRouterConfig::load(&path).unwrap();
        let shipped = ModelRouterConfig::default();

        assert_eq!(config.switch_back.after_seconds, 60);
        assert_eq!(config.switch_back.bar, shipped.switch_back.bar);
        assert_eq!(config.limit, shipped.limit);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn an_unknown_field_or_an_out_of_range_value_is_an_error() {
        let cases = [
            ("typo", r#"{ "pick_confidnce": 0.3 }"#, "pick_confidnce"),
            (
                "bar",
                r#"{ "switch_back": { "bar": { "at": 1.5 } } }"#,
                "outside [0, 1]",
            ),
            ("pick", r#"{ "pick_confidence": -0.1 }"#, "outside [0, 1]"),
            ("zero", r#"{ "max_candidates": 0 }"#, "max_candidates"),
            ("many", r#"{ "max_candidates": 33 }"#, "max_candidates"),
            (
                "empty",
                r#"{ "limit": { "messages": [" "] } }"#,
                "empty word",
            ),
        ];

        for (name, json, expected) in cases {
            let path = yours(name, json);
            let error = ModelRouterConfig::load(&path).unwrap_err();

            assert!(error.contains(expected), "{name}: {error}");
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn your_words_are_normalized_like_the_errors_they_match() {
        let path = yours(
            "words",
            r#"{ "limit": { "messages": ["Try Again Later"], "types": ["Slow-Down"] } }"#,
        );
        let config = ModelRouterConfig::load(&path).unwrap();

        assert!(config.is_limit_error("provider.slow down", None, ""));
        assert!(config.is_limit_error("internal", None, "please try again later"));
        // The list replaced the shipped phrases.
        assert!(!config.is_limit_error("internal", None, "insufficient funds"));
        std::fs::remove_file(path).unwrap();
    }
}
