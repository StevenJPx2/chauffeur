//! Anthropic provider plugin: static equivalence tiers.

use chauffeur_core::{Provider, Tier};

const TIERS: &[(&str, Tier)] = &[
    ("claude-fable-5-1", Tier::Frontier),
    ("claude-opus-5-5", Tier::Frontier),
    ("claude-sonnet-5", Tier::Balanced),
    ("claude-haiku-4-5-20251001", Tier::Fast),
];

pub struct AnthropicProvider;

impl Provider for AnthropicProvider {
    fn id(&self) -> &str {
        "anthropic"
    }

    fn tier(&self, model: &str) -> Option<Tier> {
        TIERS
            .iter()
            .find(|(id, _)| *id == model)
            .map(|(_, tier)| *tier)
    }
}
