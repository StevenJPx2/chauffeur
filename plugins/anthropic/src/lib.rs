//! Anthropic provider plugin: equivalence tiers, by model and thinking
//! variant, shipped in `skills/config/providers/anthropic.json`.

use std::path::Path;

use chauffeur_capability_model_router::{Provider, TierEntry, TierTable};

/// The shipped tier table, compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/providers/anthropic.json");

pub struct AnthropicProvider {
    tiers: Vec<TierEntry>,
}

impl AnthropicProvider {
    /// The shipped table, replaced by your `providers/anthropic.json` at
    /// `path` when it names `tiers`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, or out of bounds.
    pub fn load(path: &Path) -> Result<Self, String> {
        let table = TierTable::load(SHIPPED, path)?;

        Ok(Self { tiers: table.tiers })
    }
}

impl Default for AnthropicProvider {
    fn default() -> Self {
        let table = TierTable::parse(SHIPPED).expect("shipped anthropic tiers are valid");

        Self { tiers: table.tiers }
    }
}

impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn tiers(&self) -> &[TierEntry] {
        &self.tiers
    }
}

#[cfg(test)]
mod tests {
    use chauffeur_capability_model_router::Tier;

    use super::*;

    fn yours(name: &str, json: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "chauffeur-anthropic-{}-{name}.json",
            std::process::id()
        ));

        std::fs::write(&path, json).unwrap();
        path
    }

    fn row(model: &str, variant: Option<&str>, tier: Tier) -> TierEntry {
        TierEntry {
            model: model.into(),
            variant: variant.map(Into::into),
            tier,
        }
    }

    #[test]
    fn opus_tiers_follow_its_thinking_variant() {
        let provider = AnthropicProvider::default();

        assert_eq!(
            provider.tier("claude-opus-5-5", Some("high")),
            Some(Tier::Frontier)
        );
        assert_eq!(
            provider.tier("claude-opus-5-5", Some("low")),
            Some(Tier::Balanced)
        );
        assert_eq!(provider.tier("claude-opus-5-5", Some("max")), None);
        assert_eq!(
            provider.tier("claude-sonnet-4-6", Some("high")),
            Some(Tier::Fast)
        );
        assert_eq!(
            provider.variants("claude-opus-5-5"),
            vec![Some("high"), Some("low")]
        );
    }

    #[test]
    fn the_shipped_file_holds_the_previous_table() {
        let provider = AnthropicProvider::load(Path::new("/nonexistent/anthropic.json")).unwrap();

        assert_eq!(
            provider.tiers(),
            [
                row("claude-opus-5-5", Some("high"), Tier::Frontier),
                row("claude-opus-5-5", Some("low"), Tier::Balanced),
                row("claude-sonnet-4-6", None, Tier::Fast),
            ]
        );
    }

    #[test]
    fn your_table_replaces_the_shipped_one() {
        let path = yours(
            "replace",
            r#"{ "tiers": [ { "model": "claude-opus-5-5", "variant": null, "tier": "fast" } ] }"#,
        );
        let provider = AnthropicProvider::load(&path).unwrap();

        assert_eq!(
            provider.tier("claude-opus-5-5", Some("high")),
            Some(Tier::Fast)
        );
        assert_eq!(provider.tier("claude-sonnet-4-6", None), None);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn an_invalid_table_is_an_error() {
        let cases = [
            ("field", r#"{ "tiers": [], "extra": 1 }"#, "extra"),
            (
                "tier",
                r#"{ "tiers": [ { "model": "m", "variant": null, "tier": "huge" } ] }"#,
                "huge",
            ),
            (
                "empty",
                r#"{ "tiers": [ { "model": " ", "variant": null, "tier": "fast" } ] }"#,
                "empty model",
            ),
            (
                "twice",
                r#"{ "tiers": [ { "model": "m", "variant": null, "tier": "fast" },
                               { "model": "m", "variant": null, "tier": "frontier" } ] }"#,
                "repeats m at any variant",
            ),
        ];

        for (name, json, expected) in cases {
            let path = yours(name, json);
            let error = AnthropicProvider::load(&path).err().unwrap();

            assert!(error.contains(expected), "{name}: {error}");
            std::fs::remove_file(path).unwrap();
        }
    }
}
