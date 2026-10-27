use std::collections::BTreeSet;

use super::error::QuestionError;
use super::label::Label;

/// The fewest and most outcomes a question may have.
const OUTCOMES: std::ops::RangeInclusive<usize> = 2..=64;

/// Names a judge's result can take that no question may use as an outcome.
const RESERVED: [&str; 2] = ["unsure", "failed"];

/// A question's outcomes, in order: 2 to 64, unique, none reserved. The order
/// is a scale's order when the outcomes are one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcomes(Vec<Label>);

impl Outcomes {
    /// Validates `labels` as a question's outcomes.
    ///
    /// # Errors
    ///
    /// A [`QuestionError`] naming the first problem found.
    pub fn new(labels: impl IntoIterator<Item = impl Into<Label>>) -> Result<Self, QuestionError> {
        let labels: Vec<Label> = labels.into_iter().map(Into::into).collect();
        if !OUTCOMES.contains(&labels.len()) {
            return Err(QuestionError::OutcomeCount(labels.len()));
        }
        let mut seen = BTreeSet::new();
        for label in &labels {
            let value = label.value();
            if value.trim().is_empty() {
                return Err(QuestionError::BlankOutcome);
            }
            if RESERVED.contains(&value) {
                return Err(QuestionError::Reserved(value.to_owned()));
            }
            if !seen.insert(value) {
                return Err(QuestionError::Duplicate(value.to_owned()));
            }
        }
        Ok(Self(labels))
    }

    /// How many outcomes there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Always false: a question has at least two outcomes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The outcomes, in order.
    pub fn iter(&self) -> impl Iterator<Item = &Label> {
        self.0.iter()
    }

    /// The position of the outcome whose value is `value`.
    #[must_use]
    pub fn position(&self, value: &str) -> Option<usize> {
        self.0.iter().position(|label| label.value() == value)
    }

    /// The outcome at `index`.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Label> {
        self.0.get(index)
    }
}
