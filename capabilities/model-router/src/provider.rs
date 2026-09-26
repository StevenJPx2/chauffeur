//! Model-provider plugin contract.

use std::path::Path;

use chauffeur_core::load_layered;
use serde::{Deserialize, Serialize};

/// Rows a tier table may hold.
pub const MAX_TIER_ROWS: usize = 256;

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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TierEntry {
    pub model: String,
    /// `None` covers every variant the table does not name.
    pub variant: Option<String>,
    pub tier: Tier,
}

/// A provider's tier file: `{ "tiers": [ … ] }`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TierTable {
    pub tiers: Vec<TierEntry>,
}

impl TierTable {
    /// A table from JSON, such as a plugin's shipped file.
    ///
    /// # Errors
    ///
    /// When the JSON is invalid or out of bounds.
    pub fn parse(json: &str) -> Result<Self, String> {
        serde_json::from_str::<Self>(json)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
    }

    /// The `shipped` table, replaced by your file at `path` when it names
    /// `tiers`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, or out of bounds.
    pub fn load(shipped: &str, path: &Path) -> Result<Self, String> {
        load_layered::<Self>(shipped, path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The table, if every model is named, each (model, variant) row is
    /// unique, and it has at most [`MAX_TIER_ROWS`] rows.
    ///
    /// # Errors
    ///
    /// Names the first row out of bounds.
    pub fn checked(self) -> Result<Self, String> {
        if self.tiers.len() > MAX_TIER_ROWS {
            return Err(format!("more than {MAX_TIER_ROWS} tier rows"));
        }

        for (index, entry) in self.tiers.iter().enumerate() {
            if entry.model.trim().is_empty() {
                return Err(format!("tier row {index} has an empty model"));
            }

            if entry
                .variant
                .as_deref()
                .is_some_and(|v| v.trim().is_empty())
            {
                return Err(format!("tier row {index} has an empty variant"));
            }

            let repeated = self.tiers[..index]
                .iter()
                .any(|earlier| earlier.model == entry.model && earlier.variant == entry.variant);

            if repeated {
                return Err(format!(
                    "tier row {index} repeats {} at {}",
                    entry.model,
                    entry.variant.as_deref().unwrap_or("any variant")
                ));
            }
        }

        Ok(self)
    }
}

/// A provider plugin ships a tier table for its models.
pub trait Provider: Send + Sync {
    /// Host provider ID, e.g. `anthropic`.
    fn id(&self) -> &str;

    fn tiers(&self) -> &[TierEntry];

    /// The exact variant's tier, else the model's any-variant tier.
    fn tier(&self, model: &str, variant: Option<&str>) -> Option<Tier> {
        let rows = self.tiers().iter().filter(|entry| entry.model == model);
        let exact = rows
            .clone()
            .find(|entry| variant.is_some() && entry.variant.as_deref() == variant);

        exact
            .or_else(|| rows.clone().find(|entry| entry.variant.is_none()))
            .map(|entry| entry.tier)
    }

    /// The variants a switch to `model` may choose: each variant the table
    /// names, and the default when an any-variant row exists.
    fn variants(&self, model: &str) -> Vec<Option<&str>> {
        let mut variants: Vec<Option<&str>> = Vec::new();

        for entry in self.tiers().iter().filter(|entry| entry.model == model) {
            let variant = entry.variant.as_deref();

            if !variants.contains(&variant) {
                variants.push(variant);
            }
        }

        variants
    }
}
