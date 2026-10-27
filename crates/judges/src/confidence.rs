use std::collections::BTreeMap;
use std::fmt;

use chauffeur_core::Failure;

use crate::question::{Outcomes, Question};

/// A probability in thousandths, from 0 to 1000, so comparisons stay exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Milli(u16);

impl Milli {
    /// Certainty.
    pub const ONE: Self = Self(1000);

    /// `thousandths` as a probability, if it is at most 1000.
    #[must_use]
    pub const fn new(thousandths: u16) -> Option<Self> {
        if thousandths <= 1000 {
            Some(Self(thousandths))
        } else {
            None
        }
    }

    /// `p`, from 0 to 1, rounded to the nearest thousandth.
    #[must_use]
    pub fn from_f64(p: f64) -> Option<Self> {
        if !(0.0..=1.0).contains(&p) {
            return None;
        }
        let rounded = (p * 1000.0).round();
        #[expect(
            clippy::as_conversions,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "`rounded` is a whole number from 0 to 1000, checked above"
        )]
        let thousandths = rounded as u16;
        Self::new(thousandths)
    }

    /// The probability in thousandths.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The probability, from 0 to 1.
    #[must_use]
    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / 1000.0
    }
}

/// A probability for each of a question's outcomes, in the outcomes' order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probabilities(Vec<Milli>);

impl Probabilities {
    /// Reads a judge's reply: exactly one probability, from 0 to 1, for every
    /// outcome, and not all of them zero.
    ///
    /// # Errors
    ///
    /// [`Failure::Invalid`] for a missing, repeated, unknown or out-of-range
    /// outcome, or when every probability is zero.
    pub fn read<'a>(
        outcomes: &Outcomes,
        reply: impl IntoIterator<Item = (&'a str, f64)>,
    ) -> Result<Self, Failure> {
        let mut slots: Vec<Option<Milli>> = vec![None; outcomes.len()];
        for (value, p) in reply {
            let slot = outcomes
                .position(value)
                .and_then(|index| slots.get_mut(index));
            match (slot, Milli::from_f64(p)) {
                (Some(slot @ None), Some(p)) => *slot = Some(p),
                _ => return Err(Failure::Invalid),
            }
        }
        let read: Vec<Milli> = slots
            .into_iter()
            .collect::<Option<_>>()
            .ok_or(Failure::Invalid)?;
        if read.iter().all(|p| p.0 == 0) {
            return Err(Failure::Invalid);
        }
        Ok(Self(read))
    }

    /// The probabilities, in the outcomes' order.
    #[must_use]
    pub fn as_slice(&self) -> &[Milli] {
        &self.0
    }

    /// The probability-weighted position in the outcome list, from 0 (first)
    /// to n − 1 (last).
    #[must_use]
    pub fn level(&self) -> f64 {
        let (mut weighted, mut total, mut position) = (0.0_f64, 0.0_f64, 0.0_f64);
        for p in &self.0 {
            let p = f64::from(p.0);
            weighted = position.mul_add(p, weighted);
            total += p;
            position += 1.0;
        }
        weighted / total
    }
}

/// How likely an outcome must be to settle. Below it, the answer is `unsure`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confidence {
    /// The likeliest outcome settles; a tie is `unsure`.
    Likeliest,
    /// The likeliest outcome settles when it is at least this likely.
    Bar(Milli),
    /// The likeliest outcome settles when it is at least as likely as its own
    /// bar. Must cover every outcome.
    PerOutcome(BTreeMap<String, Milli>),
}

/// Why a confidence does not fit a question.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfidenceError {
    /// The per-outcome confidence lacks this outcome.
    #[error("the confidence has no bar for the outcome `{0}`")]
    Missing(String),
    /// The per-outcome confidence names an outcome the question lacks.
    #[error("the confidence names `{0}`, which is not an outcome")]
    Unknown(String),
}

impl Confidence {
    /// Checks that a per-outcome confidence covers exactly `outcomes`.
    ///
    /// # Errors
    ///
    /// The first outcome it lacks, or the first name that is not an outcome.
    pub fn check(&self, outcomes: &Outcomes) -> Result<(), ConfidenceError> {
        let Self::PerOutcome(bars) = self else {
            return Ok(());
        };
        if let Some(label) = outcomes
            .iter()
            .find(|label| !bars.contains_key(label.value()))
        {
            return Err(ConfidenceError::Missing(label.value().to_owned()));
        }
        if let Some(name) = bars.keys().find(|name| outcomes.position(name).is_none()) {
            return Err(ConfidenceError::Unknown(name.clone()));
        }
        Ok(())
    }

    /// The bar for the outcome at `index`; none means it never settles.
    fn bar(&self, outcomes: &Outcomes, index: usize) -> Option<Milli> {
        match self {
            Self::Likeliest => Some(Milli(0)),
            Self::Bar(bar) => Some(*bar),
            Self::PerOutcome(bars) => outcomes
                .get(index)
                .and_then(|label| bars.get(label.value()))
                .copied(),
        }
    }
}

/// The outcome at the index that settles, or none for `unsure`: the likeliest
/// outcome settles when no other is as likely and it meets its bar, edges
/// included.
pub fn pick(p: &[Milli], bar: impl Fn(usize) -> Option<Milli>) -> Option<usize> {
    let (top, best) = p.iter().enumerate().max_by_key(|(_, p)| **p)?;
    let tied = p.iter().filter(|p| *p == best).count() > 1;
    let meets = bar(top).is_some_and(|bar| *best >= bar);
    (!tied && meets).then_some(top)
}

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
