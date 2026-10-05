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
    /// A model ID, or a family pattern where `*` matches any run of
    /// characters, such as `claude-opus-*`, so new versions need no new row.
    pub model: String,
    /// `None` covers every variant the table does not name.
    pub variant: Option<String>,
    pub tier: Tier,
    /// A model you recommend: offered before other models of its tier, and
    /// named as recommended to System One. Only a row naming the model
    /// exactly may recommend it; a family pattern stays a fallback.
    #[serde(default)]
    pub recommended: bool,
}

/// How a table rates one model at one variant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rating {
    pub tier: Tier,
    pub recommended: bool,
}

/// A provider's tier file: `{ "tiers": [ … ], "exclude": [ … ] }`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TierTable {
    pub tiers: Vec<TierEntry>,
    /// Model patterns no family pattern covers, such as `*-fast` editions or
    /// older generations: such a model has a tier only through a row naming
    /// it exactly, so failover never reaches it otherwise.
    #[serde(default)]
    pub exclude: Vec<String>,
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
    /// unique, only exact rows recommend, and it has at most
    /// [`MAX_TIER_ROWS`] rows and exclusions.
    ///
    /// # Errors
    ///
    /// Names the first row out of bounds.
    pub fn checked(self) -> Result<Self, String> {
        if self.tiers.len() > MAX_TIER_ROWS || self.exclude.len() > MAX_TIER_ROWS {
            return Err(format!("more than {MAX_TIER_ROWS} tier rows or exclusions"));
        }

        if self.exclude.iter().any(|pattern| pattern.trim().is_empty()) {
            return Err("an empty exclude pattern".into());
        }

        for (index, entry) in self.tiers.iter().enumerate() {
            if entry.model.trim().is_empty() {
                return Err(format!("tier row {index} has an empty model"));
            }

            if entry.recommended && entry.model.contains('*') {
                return Err(format!(
                    "tier row {index} recommends the pattern {}; name the model exactly",
                    entry.model
                ));
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

    /// Patterns family rows do not cover; see [`TierTable::exclude`].
    fn excluded(&self) -> &[String] {
        &[]
    }

    /// The exact variant's row, else the model's any-variant row. Rows
    /// naming the model exactly win over family patterns.
    fn rating(&self, model: &str, variant: Option<&str>) -> Option<Rating> {
        let rows = rows_for(self.tiers(), self.excluded(), model);
        let exact = rows
            .iter()
            .find(|entry| variant.is_some() && entry.variant.as_deref() == variant);

        exact
            .or_else(|| rows.iter().find(|entry| entry.variant.is_none()))
            .map(|entry| Rating {
                tier: entry.tier,
                recommended: entry.recommended,
            })
    }

    fn tier(&self, model: &str, variant: Option<&str>) -> Option<Tier> {
        self.rating(model, variant).map(|rating| rating.tier)
    }

    /// The variants a switch to `model` may choose: each variant the table
    /// names, and the default when an any-variant row exists.
    fn variants(&self, model: &str) -> Vec<Option<&str>> {
        let mut variants: Vec<Option<&str>> = Vec::new();

        for entry in rows_for(self.tiers(), self.excluded(), model) {
            let variant = entry.variant.as_deref();

            if !variants.contains(&variant) {
                variants.push(variant);
            }
        }

        variants
    }
}

/// The rows for `model`: those naming it exactly, else, unless `excluded`
/// matches it, those whose pattern matches it.
fn rows_for<'a>(tiers: &'a [TierEntry], excluded: &[String], model: &str) -> Vec<&'a TierEntry> {
    let exact: Vec<&TierEntry> = tiers.iter().filter(|entry| entry.model == model).collect();

    if !exact.is_empty() || excluded.iter().any(|pattern| glob(pattern, model)) {
        return exact;
    }

    tiers
        .iter()
        .filter(|entry| entry.model.contains('*') && glob(&entry.model, model))
        .collect()
}

/// Whether `text` matches `pattern`, where `*` matches any run of characters.
#[must_use]
pub fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or_default();

    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();

    let Some((last, middle)) = parts.split_last() else {
        // No `*`: the whole text must have been the prefix.
        return rest.is_empty();
    };

    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }

    rest.len() >= last.len() && rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::glob;

    #[test]
    fn patterns_match_model_families() {
        assert!(glob("claude-opus-*", "claude-opus-5-5"));
        assert!(glob("claude-opus-*", "claude-opus-5-5-fast"));
        assert!(!glob("claude-opus-*", "claude-sonnet-5-5"));
        assert!(glob("gpt-*-sol*", "gpt-6.1-sol"));
        assert!(glob("gpt-*-sol*", "gpt-6-sol-fast"));
        assert!(!glob("gpt-*-sol*", "gpt-6-luna"));
        assert!(glob("exact", "exact"));
        assert!(!glob("exact", "exactly"));
        assert!(!glob("a*a", "a"));
    }
}
