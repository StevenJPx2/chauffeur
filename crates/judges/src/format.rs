use std::collections::BTreeMap;

use chauffeur_core::Failure;

use crate::confidence::Probabilities;
use crate::question::Question;

/// A navigation's named evidence blocks, already redacted. A question names
/// the blocks its judge sees.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence(BTreeMap<String, String>);

impl Evidence {
    /// Evidence from named blocks.
    pub fn new(blocks: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>) -> Self {
        Self(
            blocks
                .into_iter()
                .map(|(name, content)| (name.into(), content.into()))
                .collect(),
        )
    }

    /// The block named `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    /// The blocks `questions` name, each once, in name order.
    pub fn named_by<'a>(
        &'a self,
        questions: &[&Question],
    ) -> impl Iterator<Item = (&'a str, &'a str)> {
        let mut names: Vec<&str> = questions
            .iter()
            .flat_map(|question| question.evidence())
            .map(String::as_str)
            .collect();
        names.sort_unstable();
        names.dedup();
        names
            .into_iter()
            .filter_map(|name| self.0.get_key_value(name))
            .map(|(name, content)| (name.as_str(), content.as_str()))
    }
}

/// Speaks one model API: asks every question of a round in one call, and
/// returns a probability for each outcome, or why it could not.
pub trait Format: Send + Sync {
    /// One result per question, in order.
    fn answer_all(
        &self,
        questions: &[&Question],
        evidence: &Evidence,
    ) -> Vec<Result<Probabilities, Failure>>;

    /// Whether its answers only stand in for a real model's.
    fn is_placeholder(&self) -> bool {
        false
    }
}
