mod answers;
mod entry;

use std::collections::BTreeMap;

use chauffeur_core::Failure;
pub use entry::PlaceholderEntry;

use crate::confidence::Probabilities;
use crate::config::ConfigError;
use crate::format::{Evidence, Format};
use crate::question::Question;
use answers::{Recorded, Reply};

/// Stands in for a real model: recorded answers matched by evidence, the
/// first match winning, then `otherwise`. With neither, a question fails as
/// unavailable, so a gap in the recording shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Placeholder {
    answers: Vec<Recorded>,
    otherwise: Option<BTreeMap<String, f64>>,
}

impl Placeholder {
    /// Loads an entry, reading its answers file if it names one.
    ///
    /// # Errors
    ///
    /// The answers file cannot be read or parsed, or an answer gives both or
    /// neither of `p` and `failed`, or an unknown failure.
    pub fn load(entry: &PlaceholderEntry) -> Result<Self, ConfigError> {
        let answers = match &entry.answers {
            Some(path) => answers::read(path)?,
            None => Vec::new(),
        };
        Ok(Self {
            answers,
            otherwise: entry.otherwise.clone(),
        })
    }

    fn answer(&self, question: &Question, evidence: &Evidence) -> Result<Probabilities, Failure> {
        let recorded = self
            .answers
            .iter()
            .find(|recorded| recorded.matches(question, evidence));
        let p = match recorded.map(|recorded| &recorded.reply) {
            Some(Reply::P(p)) => p,
            Some(Reply::Failed(failure)) => return Err(*failure),
            None => self.otherwise.as_ref().ok_or(Failure::Unavailable)?,
        };
        Probabilities::read(
            question.outcomes(),
            p.iter().map(|(value, p)| (value.as_str(), *p)),
        )
    }
}

impl Format for Placeholder {
    fn answer_all(
        &self,
        questions: &[&Question],
        evidence: &Evidence,
    ) -> Vec<Result<Probabilities, Failure>> {
        questions
            .iter()
            .map(|question| self.answer(question, evidence))
            .collect()
    }

    fn is_placeholder(&self) -> bool {
        true
    }
}
