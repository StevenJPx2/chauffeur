use chauffeur_core::{Failure, Judge, Outcome};

use super::Fallback;
use crate::confidence::{Answer, Confidence};
use crate::format::Evidence;
use crate::question::Question;

/// A named judge for one run: a tier's models, the confidence that settles
/// their answers, and the run's evidence. Core sees it as a [`Judge`].
pub struct NamedJudge<'a> {
    models: &'a Fallback,
    confidence: &'a Confidence,
    evidence: &'a Evidence,
}

impl<'a> NamedJudge<'a> {
    /// A judge asking `models`, settling with `confidence`, over `evidence`.
    #[must_use]
    pub const fn new(
        models: &'a Fallback,
        confidence: &'a Confidence,
        evidence: &'a Evidence,
    ) -> Self {
        Self {
            models,
            confidence,
            evidence,
        }
    }
}

impl Judge<Question, Answer> for NamedJudge<'_> {
    fn judge(&self, request: &Question) -> Outcome<Answer> {
        self.judge_all(&[request])
            .pop()
            .unwrap_or(Outcome::Failed(Failure::Invalid))
    }

    fn judge_all(&self, requests: &[&Question]) -> Vec<Outcome<Answer>> {
        let replies = self.models.answer_all(requests, self.evidence);
        requests
            .iter()
            .zip(replies)
            .map(|(question, reply)| match reply {
                Ok((p, by)) => Outcome::Value(Answer::settle(question, p, self.confidence, by)),
                Err(failure) => Outcome::Failed(failure),
            })
            .collect()
    }
}
