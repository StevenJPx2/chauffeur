use chauffeur_core::Failure;

use super::milli::Milli;
use crate::question::Outcomes;

/// A probability for each of a question's outcomes, in the outcomes' order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probabilities(pub(super) Vec<Milli>);

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
