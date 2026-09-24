//! Anthropic provider plugin: static equivalence tiers, by model and
//! thinking variant.

use chauffeur_core::{Provider, Tier, TierEntry};

const TIERS: &[TierEntry] = &[
    TierEntry {
        model: "claude-opus-5-5",
        variant: Some("high"),
        tier: Tier::Frontier,
    },
    TierEntry {
        model: "claude-opus-5-5",
        variant: Some("low"),
        tier: Tier::Balanced,
    },
    TierEntry {
        model: "claude-sonnet-4-6",
        variant: None,
        tier: Tier::Fast,
    },
];

pub struct AnthropicProvider;

impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn tiers(&self) -> &[TierEntry] {
        TIERS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_tiers_follow_its_thinking_variant() {
        let provider = AnthropicProvider;

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
}
