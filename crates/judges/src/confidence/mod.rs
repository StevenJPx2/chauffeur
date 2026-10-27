mod answer;
mod error;
mod milli;
mod probabilities;

use std::collections::BTreeMap;

pub use answer::{Answer, AnsweredBy, Settled};
pub use error::ConfidenceError;
pub use milli::Milli;
pub use probabilities::Probabilities;

use crate::question::Outcomes;

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
