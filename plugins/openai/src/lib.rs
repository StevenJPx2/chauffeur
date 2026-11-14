//! OpenAI provider plugin: equivalence tiers, by model and thinking variant,
//! shipped in `skills/config/providers/openai.json`, and [`codex`], a System
//! Two backend over your ChatGPT subscription.

pub mod codex;

use std::path::Path;

use chauffeur_capability_model_router::{Provider, TierEntry, TierTable};

/// The shipped tier table, compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/providers/openai.json");

pub struct OpenAiProvider {
    table: TierTable,
}

impl OpenAiProvider {
    /// The shipped table, with your `providers/openai.json` at `path`
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

impl Default for OpenAiProvider {
    fn default() -> Self {
        Self {
            table: TierTable::parse(SHIPPED).expect("shipped openai tiers are valid"),
        }
    }
}

impl Provider for OpenAiProvider {
    fn id(&self) -> &str {
        "openai"
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
    fn each_family_has_one_tier_at_every_variant() {
        let provider = OpenAiProvider::default();
        let tier = |model: &str, variant: Option<&str>| provider.tier(model, variant);

        assert_eq!(tier("gpt-6-astra", Some("high")), Some(Tier::Frontier));
        assert_eq!(tier("gpt-6-astra", Some("low")), Some(Tier::Frontier));
        assert_eq!(tier("gpt-6.1-sol", Some("high")), Some(Tier::Balanced));
        assert_eq!(tier("gpt-6.1-sol", Some("low")), Some(Tier::Balanced));
        assert_eq!(tier("gpt-6.1-terra", Some("high")), Some(Tier::Efficient));
        assert_eq!(tier("gpt-6.1-terra", Some("max")), Some(Tier::Efficient));
        assert_eq!(tier("gpt-6.1-terra", None), Some(Tier::Efficient));
        // Luna is fast at every variant, max included.
        assert_eq!(tier("gpt-6-luna", Some("max")), Some(Tier::Fast));
        assert_eq!(tier("gpt-6-luna", None), Some(Tier::Fast));
        // A switch picks the first variant a model's rows name.
        assert_eq!(provider.variants("gpt-6-astra"), vec![Some("high"), None]);
        assert_eq!(provider.variants("gpt-6.1-sol"), vec![Some("high"), None]);
        assert_eq!(provider.variants("gpt-6.1-terra"), vec![Some("high"), None]);
        assert_eq!(provider.variants("gpt-6-luna"), vec![None]);
        // Family patterns cover new versions.
        assert_eq!(tier("gpt-6.1-astra", None), Some(Tier::Frontier));
        assert_eq!(tier("gpt-6-sol", Some("high")), Some(Tier::Balanced));
        assert_eq!(tier("gpt-6.2-terra", Some("high")), Some(Tier::Efficient));
        assert_eq!(tier("gpt-6.1-luna", Some("max")), Some(Tier::Fast));
        // Fast editions and older generations have no tier.
        assert_eq!(tier("gpt-6-astra-fast", Some("high")), None);
        assert_eq!(tier("gpt-6.1-sol-fast", Some("high")), None);
        assert_eq!(tier("gpt-6.1-terra-fast", Some("high")), None);
        assert_eq!(tier("gpt-5.6-sol", None), None);
    }

    #[test]
    fn only_the_named_current_models_are_recommended() {
        let provider = OpenAiProvider::default();
        let rated = |model: &str, variant: Option<&str>| {
            provider
                .rating(model, variant)
                .map(|rating| rating.recommended)
        };

        assert_eq!(rated("gpt-6-astra", Some("high")), Some(true));
        assert_eq!(rated("gpt-6.1-sol", Some("high")), Some(true));
        assert_eq!(rated("gpt-6.1-terra", Some("high")), Some(true));
        assert_eq!(rated("gpt-6-luna", None), Some(true));
        assert_eq!(rated("gpt-6-astra", None), Some(false));
        assert_eq!(rated("gpt-6.1-terra", Some("low")), Some(false));
        assert_eq!(rated("gpt-6-sol", Some("high")), Some(false));
    }

    #[test]
    fn the_shipped_file_holds_the_previous_table() {
        let provider = OpenAiProvider::load(Path::new("/nonexistent/openai.json")).unwrap();

        assert_eq!(
            provider.tiers(),
            [
                recommended("gpt-6-astra", Some("high"), Tier::Frontier),
                row("gpt-6-astra", None, Tier::Frontier),
                recommended("gpt-6.1-sol", Some("high"), Tier::Balanced),
                row("gpt-6.1-sol", None, Tier::Balanced),
                recommended("gpt-6.1-terra", Some("high"), Tier::Efficient),
                row("gpt-6.1-terra", None, Tier::Efficient),
                recommended("gpt-6-luna", None, Tier::Fast),
                row("gpt-*-astra*", Some("high"), Tier::Frontier),
                row("gpt-*-astra*", None, Tier::Frontier),
                row("gpt-*-sol*", Some("high"), Tier::Balanced),
                row("gpt-*-sol*", None, Tier::Balanced),
                row("gpt-*-terra*", Some("high"), Tier::Efficient),
                row("gpt-*-terra*", None, Tier::Efficient),
                row("gpt-*-luna*", None, Tier::Fast),
            ]
        );
    }
}
