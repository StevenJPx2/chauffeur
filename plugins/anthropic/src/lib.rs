//! Anthropic provider plugin: equivalence tiers, by model and thinking
//! variant, shipped in `skills/config/providers/anthropic.json`.

use std::path::Path;

use chauffeur_capability_model_router::{Provider, TierEntry, TierTable};

/// The shipped tier table, compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/providers/anthropic.json");

pub struct AnthropicProvider {
    table: TierTable,
}

impl AnthropicProvider {
    /// The shipped table, with your `providers/anthropic.json` at `path`
    /// replacing whichever of `tiers` and `exclude` it names.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, or out of bounds.
    pub fn load(path: &Path) -> Result<Self, String> {
        Ok(Self {
            table: TierTable::load(SHIPPED, path)?,
        })
    }
}

impl Default for AnthropicProvider {
    fn default() -> Self {
        Self {
            table: TierTable::parse(SHIPPED).expect("shipped anthropic tiers are valid"),
        }
    }
}

impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn tiers(&self) -> &[TierEntry] {
        &self.table.tiers
    }

    fn excluded(&self) -> &[String] {
        &self.table.exclude
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
            recommended: false,
        }
    }

    fn recommended(model: &str, variant: Option<&str>, tier: Tier) -> TierEntry {
        TierEntry {
            recommended: true,
            ..row(model, variant, tier)
        }
    }

    #[test]
    fn tiers_follow_the_family_and_opus_its_thinking_variant() {
        let provider = AnthropicProvider::default();
        let tier = |model: &str, variant: Option<&str>| provider.tier(model, variant);

        // Fable is frontier at every variant.
        assert_eq!(tier("claude-fable-5-2", Some("high")), Some(Tier::Frontier));
        assert_eq!(tier("claude-fable-5-2", Some("low")), Some(Tier::Frontier));
        assert_eq!(tier("claude-fable-5-2", None), Some(Tier::Frontier));
        // Opus is efficient at low thinking and balanced otherwise.
        assert_eq!(tier("claude-opus-5-5", Some("high")), Some(Tier::Balanced));
        assert_eq!(tier("claude-opus-5-5", Some("max")), Some(Tier::Balanced));
        assert_eq!(tier("claude-opus-5-5", None), Some(Tier::Balanced));
        assert_eq!(tier("claude-opus-5-5", Some("low")), Some(Tier::Efficient));
        // Sonnet is efficient at every variant; Haiku fast.
        assert_eq!(
            tier("claude-sonnet-5-5", Some("high")),
            Some(Tier::Efficient)
        );
        assert_eq!(tier("claude-sonnet-5-5", None), Some(Tier::Efficient));
        assert_eq!(tier("claude-haiku-5", None), Some(Tier::Fast));
        // Family patterns cover versions the table does not name.
        assert_eq!(tier("claude-fable-5-3", Some("max")), Some(Tier::Frontier));
        assert_eq!(tier("claude-opus-5-6", Some("low")), Some(Tier::Efficient));
        assert_eq!(
            provider.variants("claude-fable-5-2"),
            vec![Some("high"), None]
        );
        assert_eq!(
            provider.variants("claude-opus-5-5"),
            vec![Some("high"), Some("low"), None]
        );
        assert_eq!(
            provider.variants("claude-sonnet-5-5"),
            vec![Some("high"), None]
        );
    }

    #[test]
    fn current_models_are_recommended_and_fast_or_old_editions_excluded() {
        let provider = AnthropicProvider::default();
        let rated = |model: &str, variant: Option<&str>| {
            provider
                .rating(model, variant)
                .map(|rating| rating.recommended)
        };

        assert_eq!(rated("claude-fable-5-2", Some("high")), Some(true));
        assert_eq!(rated("claude-opus-5-5", Some("high")), Some(true));
        assert_eq!(rated("claude-opus-5-5", Some("low")), Some(true));
        // Other variants keep their tier without the recommendation.
        assert_eq!(rated("claude-fable-5-2", None), Some(false));
        assert_eq!(rated("claude-opus-5-5", Some("max")), Some(false));
        // Sonnet is efficient but not recommended: an efficient failover
        // prefers Opus at low thinking, like for like with Terra.
        assert_eq!(rated("claude-sonnet-5-5", Some("high")), Some(false));
        // An older 5.x model is a fallback of the same tier, not recommended.
        assert_eq!(rated("claude-fable-5-1", Some("high")), Some(false));
        assert_eq!(provider.tier("claude-opus-5", None), Some(Tier::Balanced));
        // Fast editions and generation 4 have no tier at all.
        assert_eq!(rated("claude-fable-5-2-fast", None), None);
        assert_eq!(rated("claude-opus-5-5-fast", None), None);
        assert_eq!(rated("claude-opus-4-8", None), None);
        assert_eq!(rated("claude-sonnet-4-6", None), None);
    }

    #[test]
    fn the_shipped_file_holds_the_previous_table() {
        let provider = AnthropicProvider::load(Path::new("/nonexistent/anthropic.json")).unwrap();

        assert_eq!(
            provider.tiers(),
            [
                recommended("claude-fable-5-2", Some("high"), Tier::Frontier),
                row("claude-fable-5-2", None, Tier::Frontier),
                recommended("claude-opus-5-5", Some("high"), Tier::Balanced),
                recommended("claude-opus-5-5", Some("low"), Tier::Efficient),
                row("claude-opus-5-5", None, Tier::Balanced),
                row("claude-fable-*", Some("high"), Tier::Frontier),
                row("claude-fable-*", None, Tier::Frontier),
                row("claude-opus-*", Some("high"), Tier::Balanced),
                row("claude-opus-*", Some("low"), Tier::Efficient),
                row("claude-opus-*", None, Tier::Balanced),
                row("claude-sonnet-*", Some("high"), Tier::Efficient),
                row("claude-sonnet-*", None, Tier::Efficient),
                row("claude-haiku-*", None, Tier::Fast),
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
        assert_eq!(provider.tier("claude-sonnet-5-5", None), None);
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
            (
                "pattern",
                r#"{ "tiers": [ { "model": "m-*", "variant": null, "tier": "fast", "recommended": true } ] }"#,
                "recommends the pattern m-*",
            ),
            (
                "exclusion",
                r#"{ "tiers": [], "exclude": [" "] }"#,
                "empty exclude pattern",
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
