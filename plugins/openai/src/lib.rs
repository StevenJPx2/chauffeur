//! OpenAI provider plugin: static equivalence tiers, by model and thinking
//! variant.

use chauffeur_core::{Provider, Tier, TierEntry};

const TIERS: &[TierEntry] = &[
    TierEntry {
        model: "gpt-6-sol",
        variant: None,
        tier: Tier::Frontier,
    },
    TierEntry {
        model: "gpt-6-luna",
        variant: Some("max"),
        tier: Tier::Balanced,
    },
    TierEntry {
        model: "gpt-6-luna",
        variant: None,
        tier: Tier::Fast,
    },
];

pub struct OpenAiProvider;

impl Provider for OpenAiProvider {
    fn id(&self) -> &str {
        "openai"
    }

    fn tiers(&self) -> &[TierEntry] {
        TIERS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luna_at_max_thinking_is_balanced_and_otherwise_fast() {
        let provider = OpenAiProvider;

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
}
