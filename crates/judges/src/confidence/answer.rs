use std::fmt;

use super::milli::Milli;
use super::probabilities::Probabilities;
use super::{Confidence, pick};
use crate::question::Question;

/// What an answer settled to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settled {
    /// One of the question's outcomes.
    Outcome(String),
    /// No outcome was likely enough.
    Unsure,
}

impl fmt::Display for Settled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Outcome(value) => f.write_str(value),
            Self::Unsure => f.write_str("unsure"),
        }
    }
}

/// The model that gave an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnsweredBy {
    /// Its name in `models.json`.
    pub model: String,
    /// Whether it is a placeholder, standing in for a real model.
    pub placeholder: bool,
}

/// A judge's answer: what it settled to, every outcome's probability, its
/// level, and the model that gave it.
#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    outcome: Settled,
    p: Vec<(String, Milli)>,
    level: f64,
    by: AnsweredBy,
}

impl Answer {
    /// Settles `p`, given by `by`, for `question` under `confidence`.
    #[must_use]
    pub fn settle(
        question: &Question,
        p: Probabilities,
        confidence: &Confidence,
        by: AnsweredBy,
    ) -> Self {
        let outcomes = question.outcomes();
        let picked = pick(p.as_slice(), |index| confidence.bar(outcomes, index));
        let outcome = picked
            .and_then(|index| outcomes.get(index))
            .map_or(Settled::Unsure, |label| {
                Settled::Outcome(label.value().to_owned())
            });
        let level = p.level();
        let p = outcomes
            .iter()
            .map(|label| label.value().to_owned())
            .zip(p.0)
            .collect();
        Self {
            outcome,
            p,
            level,
            by,
        }
    }

    /// The model that gave it.
    #[must_use]
    pub const fn by(&self) -> &AnsweredBy {
        &self.by
    }

    /// What the answer settled to.
    #[must_use]
    pub const fn outcome(&self) -> &Settled {
        &self.outcome
    }

    /// Every outcome's probability, in the question's order.
    pub fn p(&self) -> impl Iterator<Item = (&str, Milli)> {
        self.p.iter().map(|(value, p)| (value.as_str(), *p))
    }

    /// The probability-weighted position in the outcome list.
    #[must_use]
    pub const fn level(&self) -> f64 {
        self.level
    }
}
