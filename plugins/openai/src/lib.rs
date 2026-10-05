//! OpenAI provider plugin: equivalence tiers, by model and thinking variant,
//! shipped in `skills/config/providers/openai.json`.

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
    fn luna_at_max_thinking_is_balanced_and_otherwise_fast() {
        let provider = OpenAiProvider::default();

        assert_eq!(
            provider.tier("gpt-6-sol", Some("high")),
            Some(Tier::Frontier)
        );
        assert_eq!(
            provider.tier("gpt-6-luna", Some("max")),
            Some(Tier::Balanced)
        );
        assert_eq!(provider.tier("gpt-6-luna", Some("low")), Some(Tier::Fast));
        assert_eq!(provider.tier("gpt-6-luna", None), Some(Tier::Fast));
        assert_eq!(provider.variants("gpt-6-luna"), vec![Some("max"), None]);
        // Family patterns cover new versions and their fast editions.
        assert_eq!(provider.tier("gpt-6.1-sol", None), Some(Tier::Frontier));
        assert_eq!(
            provider.tier("gpt-6-sol", Some("high")),
            Some(Tier::Frontier)
        );
        // Fast editions and older generations have no tier.
        assert_eq!(provider.tier("gpt-6.1-sol-fast", Some("high")), None);
        assert_eq!(provider.tier("gpt-5.6-sol", None), None);
        assert_eq!(provider.tier("gpt-6-astra", None), None);
        // Only the named current models are recommended.
        assert!(
            provider
                .rating("gpt-6.1-sol", Some("high"))
                .unwrap()
                .recommended
        );
        assert!(
            !provider
                .rating("gpt-6-sol", Some("high"))
                .unwrap()
                .recommended
        );
    }

    #[test]
    fn the_shipped_file_holds_the_previous_table() {
        let provider = OpenAiProvider::load(Path::new("/nonexistent/openai.json")).unwrap();

        assert_eq!(
            provider.tiers(),
            [
                recommended("gpt-6.1-sol", Some("high"), Tier::Frontier),
                recommended("gpt-6.1-sol", None, Tier::Frontier),
                recommended("gpt-6-luna", Some("max"), Tier::Balanced),
                recommended("gpt-6-luna", None, Tier::Fast),
                row("gpt-*-sol*", Some("high"), Tier::Frontier),
                row("gpt-*-sol*", None, Tier::Frontier),
                row("gpt-*-luna*", Some("max"), Tier::Balanced),
                row("gpt-*-luna*", None, Tier::Fast),
            ]
        );
    }
}
