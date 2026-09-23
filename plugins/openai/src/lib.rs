//! OpenAI provider plugin: static equivalence tiers.

use chauffeur_core::{Provider, Tier};

const TIERS: &[(&str, Tier)] = &[
    ("gpt-6-sol", Tier::Frontier),
    ("gpt-6-luna", Tier::Frontier),
    ("gpt-6-astra", Tier::Frontier),
    ("gpt-5.6-terra", Tier::Balanced),
    ("gpt-5.5", Tier::Balanced),
    ("gpt-5.3-codex-spark", Tier::Fast),
];

pub struct OpenAiProvider;

impl Provider for OpenAiProvider {
    fn id(&self) -> &str {
        "openai"
    }

    fn tier(&self, model: &str) -> Option<Tier> {
        TIERS
            .iter()
            .find(|(id, _)| *id == model)
            .map(|(_, tier)| *tier)
    }
}
