//! Model-provider plugin contract.

use serde::{Deserialize, Serialize};

/// Equivalence tier. Models in one tier are interchangeable for failover.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Frontier,
    Balanced,
    Fast,
}

impl Tier {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Frontier => "frontier",
            Self::Balanced => "balanced",
            Self::Fast => "fast",
        }
    }
}

/// One row of a tier table: a model, at one thinking variant or at any.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TierEntry {
    pub model: &'static str,
    /// `None` covers every variant the table does not name.
    pub variant: Option<&'static str>,
    pub tier: Tier,
}

/// A provider plugin ships a static tier table for its models.
pub trait Provider: Send + Sync {
    /// Host provider ID, e.g. `anthropic`.
    fn id(&self) -> &str;

    fn tiers(&self) -> &[TierEntry];

    /// The exact variant's tier, else the model's any-variant tier.
    fn tier(&self, model: &str, variant: Option<&str>) -> Option<Tier> {
        let rows = self.tiers().iter().filter(|entry| entry.model == model);
        let exact = rows
            .clone()
            .find(|entry| variant.is_some() && entry.variant == variant);

        exact
            .or_else(|| rows.clone().find(|entry| entry.variant.is_none()))
            .map(|entry| entry.tier)
    }

    /// The variants a switch to `model` may choose: each variant the table
    /// names, and the default when an any-variant row exists.
    fn variants(&self, model: &str) -> Vec<Option<&'static str>> {
        let mut variants: Vec<Option<&'static str>> = Vec::new();

        for entry in self.tiers().iter().filter(|entry| entry.model == model) {
            if !variants.contains(&entry.variant) {
                variants.push(entry.variant);
            }
        }

        variants
    }
}
