//! The router's tunable defaults, shipped in `skills/config/model-router.json`
//! and overlaid by your `model-router.json`.

use std::collections::BTreeMap;
use std::path::Path;

use chauffeur_core::{CachePrices, Confidence, ModelRef, Template, Threshold, load_layered};
use serde::Deserialize;

use crate::provider::glob;

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
    /// `provider/model` patterns never selected by the router, at any variant.
    /// These override pins, last-resort patterns, and remembered origins.
    pub blacklist: Vec<String>,
    /// `provider/model` patterns, such as `opencode/*-free`, offered only
    /// when no model on a provider you use directly is left.
    pub last_resort: Vec<String>,
    /// Below this, a failover choice is treated as unavailable and the
    /// posture applies.
    pub pick_confidence: Confidence,
    pub switch_back: SwitchBack,
    /// When a prompt resumes a context whose prompt cache has expired.
    pub renewal: Renewal,
    /// Options offered to System One, after ordering; keeps the choice small.
    pub max_candidates: usize,
    /// A usage limit that another model avoids.
    pub limit: ErrorWords,
    /// A limit that waiting does not clear, such as an exhausted quota or
    /// balance: the router switches without offering to stay.
    pub lasting: ErrorWords,
    /// A model that cannot serve the agent at all (no access, unknown model),
    /// as opposed to a limit that may clear.
    pub unusable: ErrorWords,
    /// The wording of the router's questions and options.
    pub texts: RouterTexts,
}

/// The router's questions to System One and the options they offer.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RouterTexts {
    /// Which model to switch to, when staying is offered: `{model}`, `{error}`.
    pub choice: Template,
    /// Which model to switch to, when the limit does not clear by waiting.
    pub choice_must_switch: Template,
    /// One option: `{model}`, `{tier}`, `{provider}`, `{note}`.
    pub option: Template,
    /// `{note}` for a recommended model.
    pub recommended_note: Template,
    /// `{note}` for a last-resort model.
    pub last_resort_note: Template,
    /// The stay option: `{model}`.
    pub stay: Template,
    /// Whether the original limit has cleared: `{minutes}`, `{model}`, `{error}`.
    pub switch_back: Template,
    /// Whether to renew a resumed context: `{model}`, `{idle_minutes}`,
    /// `{cache_minutes}`, `{tokens}`, `{window}`, `{budget}`.
    pub renew_context: Template,
}

impl RouterTexts {
    fn checked(self) -> Result<Self, String> {
        let error = &["model", "error"][..];

        for (field, template, allowed) in [
            ("texts.choice", &self.choice, error),
            ("texts.choice_must_switch", &self.choice_must_switch, error),
            (
                "texts.option",
                &self.option,
                &["model", "tier", "provider", "note"][..],
            ),
            ("texts.recommended_note", &self.recommended_note, &[][..]),
            ("texts.last_resort_note", &self.last_resort_note, &[][..]),
            ("texts.stay", &self.stay, &["model"][..]),
            (
                "texts.switch_back",
                &self.switch_back,
                &["minutes", "model", "error"][..],
            ),
            (
                "texts.renew_context",
                &self.renew_context,
                &[
                    "model",
                    "idle_minutes",
                    "cache_minutes",
                    "tokens",
                    "window",
                    "budget",
                ][..],
            ),
        ] {
            template.check(field, allowed)?;
        }

        Ok(self)
    }
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

/// Renewing a long context whose prompt cache has expired: a cheaper model
/// summarizes it before the prompt runs on the agent's own model. Whether to
/// is judged from facts, by the judge `judge.json` routes
/// `model-router/renew_context` to.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Renewal {
    /// Seconds each provider keeps an unread prompt cache, by provider ID;
    /// `*` for every other provider.
    pub cache_seconds: BTreeMap<String, u64>,
    /// How long a context grows before renewing it pays.
    pub budget: Budget,
    /// Keeping an idle context's prompt cache warm, so renewal waits until
    /// warming stops and the cache expires.
    pub warming: Warming,
    /// Preferred summary models as `provider/model`, offered before the fast
    /// tier.
    pub pins: Vec<String>,
    /// Renew on a yes at this bar.
    pub bar: Threshold,
}

/// The tokens a context may hold before it is worth renewing: a share of
/// the model's window, never under `min_tokens` nor over `max_tokens`. A
/// large window still renews at the ceiling; a small one renews sooner, but
/// not so soon that a summary costs more than it saves.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Share of the context window, above 0 and at most 1.
    pub share: f64,
    pub min_tokens: u64,
    pub max_tokens: u64,
}

impl Budget {
    /// The budget for a `window`-token model; the ceiling when the window is
    /// unknown (0).
    #[must_use]
    pub fn tokens(&self, window: u64) -> u64 {
        if window == 0 {
            return self.max_tokens;
        }

        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a window is far below 2^52 and the share is positive"
        )]
        let share = (window as f64 * self.share).round() as u64;

        share.clamp(self.min_tokens, self.max_tokens)
    }

    fn checked(self) -> Result<Self, String> {
        if !(self.share > 0.0 && self.share <= 1.0) {
            return Err("renewal.budget.share must be above 0 and at most 1".into());
        }

        if self.max_tokens == 0 || self.min_tokens > self.max_tokens {
            return Err("renewal.budget needs max_tokens above 0 and min_tokens no larger".into());
        }

        Ok(self)
    }
}

/// Warming sends a transient request on an idle session's context shortly
/// before its prompt cache would expire, so the user's next prompt re-reads
/// it at the cache price. It stops `max_seconds` after the session's own last
/// request, and sooner when more warming would cost more than re-reading the
/// context cold.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Warming {
    /// Warm for at most this long after the session's last request; 0 turns
    /// warming off.
    pub max_seconds: u64,
    /// Warm this long before the cache would expire.
    pub lead_seconds: u64,
    /// A smaller context is left to go cold: re-reading it costs little.
    pub min_tokens: u64,
}

/// Longest a session may be kept warm.
const MAX_WARMING_SECONDS: u64 = 24 * 60 * 60;

impl Warming {
    /// How often to warm a `context_tokens` context whose provider keeps an
    /// unread cache for `cache_seconds`, and how many times, at `prices`.
    /// `None` when warming is off or would not pay.
    ///
    /// Each warming request re-reads the context at the cache-read price. A
    /// prompt after the cache expired pays the cache-write price instead of
    /// that read, so warming pays while its reads together cost less than
    /// the difference. An unknown cache-read price means no known caching;
    /// an unknown cache-write price is the input price, as for a provider
    /// that does not charge for writes.
    #[must_use]
    pub fn plan(
        &self,
        cache_seconds: u64,
        context_tokens: u64,
        prices: CachePrices,
    ) -> Option<(u64, u32)> {
        if self.max_seconds == 0
            || context_tokens < self.min_tokens
            || cache_seconds <= self.lead_seconds
            || prices.cache_read == 0
        {
            return None;
        }

        let every = cache_seconds - self.lead_seconds;
        let in_time = (self.max_seconds - 1) / every;
        let write = if prices.cache_write > 0 {
            prices.cache_write
        } else {
            prices.input
        };
        let paid_for = write.saturating_sub(prices.cache_read) / prices.cache_read;
        let times = u32::try_from(in_time.min(paid_for)).unwrap_or(u32::MAX);

        (times > 0).then_some((every, times))
    }

    fn checked(self) -> Result<Self, String> {
        if self.max_seconds > MAX_WARMING_SECONDS {
            return Err(format!(
                "renewal.warming.max_seconds is at most {MAX_WARMING_SECONDS}"
            ));
        }

        Ok(self)
    }
}

impl Renewal {
    /// How long `provider` keeps an unread prompt cache.
    #[must_use]
    pub fn cache_seconds(&self, provider: &str) -> u64 {
        self.cache_seconds
            .get(provider)
            .or_else(|| self.cache_seconds.get("*"))
            .copied()
            .unwrap_or_default()
    }

    fn checked(mut self) -> Result<Self, String> {
        if !self.cache_seconds.contains_key("*") {
            return Err("renewal.cache_seconds needs a `*` entry for other providers".into());
        }

        if self.pins.len() > MAX_PINS || self.pins.iter().any(|pin| !pin.contains('/')) {
            return Err(format!(
                "renewal.pins holds at most {MAX_PINS} provider/model entries"
            ));
        }

        self.budget = self.budget.checked()?;
        self.warming = self.warming.checked()?;

        Ok(self)
    }
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
        if self.pins.len() > MAX_PINS || self.last_resort.len() > MAX_PINS {
            return Err(format!("more than {MAX_PINS} pins or last-resort patterns"));
        }

        if self.blacklist.len() > MAX_PINS {
            return Err(format!("more than {MAX_PINS} blacklist patterns"));
        }

        if self.blacklist.iter().any(|pattern| {
            !pattern.split_once('/').is_some_and(|(provider, model)| {
                !provider.trim().is_empty() && !model.trim().is_empty()
            })
        }) {
            return Err("a blacklist pattern must be provider/model, such as opencode-go/*".into());
        }

        if self
            .last_resort
            .iter()
            .any(|pattern| !pattern.contains('/'))
        {
            return Err(
                "a last-resort pattern must be provider/model, such as opencode/*-free".into(),
            );
        }

        if !(1..=MAX_CANDIDATES).contains(&self.max_candidates) {
            return Err(format!("max_candidates must be 1 to {MAX_CANDIDATES}"));
        }

        self.limit = self.limit.checked("limit")?;
        self.lasting = self.lasting.checked("lasting")?;
        self.unusable = self.unusable.checked("unusable")?;
        self.texts = self.texts.checked()?;
        self.renewal = self.renewal.checked()?;

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

    /// Whether `model` is a last-resort fallback.
    #[must_use]
    pub fn is_last_resort(&self, model: &ModelRef) -> bool {
        let key = format!("{}/{}", model.provider, model.model);

        self.last_resort.iter().any(|pattern| glob(pattern, &key))
    }

    /// Whether the router must never select `model`, regardless of variant.
    #[must_use]
    pub fn is_blacklisted(&self, model: &ModelRef) -> bool {
        let key = format!("{}/{}", model.provider, model.model);

        self.blacklist.iter().any(|pattern| glob(pattern, &key))
    }

    /// Whether the limit lasts: waiting will not clear it.
    #[must_use]
    pub fn is_lasting_error(&self, error_type: &str, status: Option<u16>, message: &str) -> bool {
        // 402 Payment Required is an exhausted balance.
        matches!(status, Some(402)) || self.lasting.matches(error_type, message)
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
        assert_eq!(config.limit.messages.len(), 13);
        assert_eq!(config.limit.types.len(), 12);
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
    fn blacklist_patterns_cover_providers_models_and_every_variant() {
        let config = ModelRouterConfig {
            blacklist: vec!["opencode-go/*".into(), "openai/gpt-*-sol".into()],
            ..ModelRouterConfig::default()
        };
        let model = |provider: &str, model: &str| ModelRef {
            provider: provider.into(),
            model: model.into(),
            variant: Some("high".into()),
        };

        assert!(config.is_blacklisted(&model("opencode-go", "example-free")));
        assert!(config.is_blacklisted(&model("openai", "gpt-6.1-sol")));
        assert!(!config.is_blacklisted(&model("opencode", "example-free")));
        assert!(!config.is_blacklisted(&model("openai", "gpt-6-luna")));
    }

    #[test]
    fn your_blacklist_replaces_the_shipped_exclusion() {
        let path = yours("blacklist", r#"{ "blacklist": [] }"#);
        let config = ModelRouterConfig::load(&path).unwrap();

        assert!(config.blacklist.is_empty());
        assert_eq!(config.last_resort, vec!["opencode/*-free"]);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn blacklist_patterns_are_bounded() {
        let config = ModelRouterConfig {
            blacklist: vec!["opencode-go/*".into(); MAX_PINS + 1],
            ..ModelRouterConfig::default()
        };

        assert!(config.checked().unwrap_err().contains("blacklist"));
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
                "blacklist-provider",
                r#"{ "blacklist": ["opencode-go"] }"#,
                "blacklist",
            ),
            ("blacklist-empty", r#"{ "blacklist": [""] }"#, "blacklist"),
            (
                "blacklist-no-provider",
                r#"{ "blacklist": ["/model"] }"#,
                "blacklist",
            ),
            (
                "blacklist-no-model",
                r#"{ "blacklist": ["provider/ "] }"#,
                "blacklist",
            ),
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
