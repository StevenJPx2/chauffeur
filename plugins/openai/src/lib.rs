//! OpenAI provider plugin: equivalence tiers, by model and thinking variant,
//! shipped in `skills/config/providers/openai.json`.

use std::path::Path;

use chauffeur_capability_model_router::{Provider, TierEntry, TierTable};

/// The shipped tier table, compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/providers/openai.json");

pub struct OpenAiProvider {
    tiers: Vec<TierEntry>,
}

impl OpenAiProvider {
    /// The shipped table, replaced by your `providers/openai.json` at `path`
    /// when it names `tiers`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, or out of bounds.
    pub fn load(path: &Path) -> Result<Self, String> {
        let table = TierTable::load(SHIPPED, path)?;

        Ok(Self { tiers: table.tiers })
    }
}

impl Default for OpenAiProvider {
    fn default() -> Self {
        let table = TierTable::parse(SHIPPED).expect("shipped openai tiers are valid");

        Self { tiers: table.tiers }
    }
}

impl Provider for OpenAiProvider {
    fn id(&self) -> &str {
        "openai"
    }

    fn tiers(&self) -> &[TierEntry] {
        &self.tiers
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
    }

    #[test]
    fn the_shipped_file_holds_the_previous_table() {
        let provider = OpenAiProvider::load(Path::new("/nonexistent/openai.json")).unwrap();

        assert_eq!(
            provider.tiers(),
            [
                row("gpt-6-sol", None, Tier::Frontier),
                row("gpt-6-luna", Some("max"), Tier::Balanced),
                row("gpt-6-luna", None, Tier::Fast),
            ]
        );
    }
}
