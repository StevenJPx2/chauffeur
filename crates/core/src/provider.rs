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

/// A provider plugin ships a static tier table for its models.
pub trait Provider: Send + Sync {
    /// Host provider ID, e.g. `anthropic`.
    fn id(&self) -> &str;

    fn tier(&self, model: &str) -> Option<Tier>;
}
