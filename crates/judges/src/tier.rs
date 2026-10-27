use chauffeur_core::{Failure, Judge, Outcome};

use crate::confidence::{Answer, AnsweredBy, Confidence, Probabilities};
use crate::config::SystemOneEntry;
use crate::config::{ConfigError, ModelEntry, Models, Tier};
use crate::format::{Evidence, Format};
use crate::placeholder::Placeholder;
use crate::question::Question;

#[cfg(feature = "systemone")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "matches the build without the `systemone` feature"
)]
fn systemone(entry: &SystemOneEntry, key: String) -> Option<Box<dyn Format>> {
    Some(Box::new(crate::systemone::SystemOne::new(entry, key)))
}

/// Built without HTTP, as for the browser: the model is left out.
#[cfg(not(feature = "systemone"))]
fn systemone(_: &SystemOneEntry, _: String) -> Option<Box<dyn Format>> {
    None
}

/// One model, ready to ask.
pub struct Model {
    name: String,
    format: Box<dyn Format>,
}

impl Model {
    /// A model named `name` that speaks `format`.
    pub fn new(name: impl Into<String>, format: Box<dyn Format>) -> Self {
        Self {
            name: name.into(),
            format,
        }
    }
}

/// A tier's models in fallback order. The next model answers only the
/// questions the one before could not: unavailable, rate-limited or timed out.
pub struct Fallback {
    models: Vec<Model>,
}

impl Fallback {
    /// Models in the order they are asked.
    #[must_use]
    pub const fn new(models: Vec<Model>) -> Self {
        Self { models }
    }

    /// The tier's models from `models`, with `preferred` asked first when it
    /// is registered. A model whose key `key` cannot find is left out, so a
    /// shipped model is used once its key is present.
    ///
    /// # Errors
    ///
    /// A placeholder's answers cannot be loaded.
    pub fn build(
        models: &Models,
        tier: Tier,
        preferred: Option<&str>,
        key: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let order = preferred
            .into_iter()
            .chain(models.tier(tier).iter().map(String::as_str));
        let mut built: Vec<Model> = Vec::new();
        for name in order {
            if built.iter().any(|model| model.name == name) {
                continue;
            }
            let format: Box<dyn Format> = match models.model(name) {
                Some(ModelEntry::Systemone(entry)) => {
                    match key(&entry.key.env).and_then(|key| systemone(entry, key)) {
                        Some(format) => format,
                        None => continue,
                    }
                }
                Some(ModelEntry::Placeholder(entry)) => Box::new(Placeholder::load(entry)?),
                None => continue,
            };
            built.push(Model::new(name, format));
        }
        Ok(Self::new(built))
    }

    /// One result per question, in order. With no model left to ask, a
    /// question fails as unavailable.
    #[must_use]
    pub fn answer_all(
        &self,
        questions: &[&Question],
        evidence: &Evidence,
    ) -> Vec<Result<(Probabilities, AnsweredBy), Failure>> {
        let mut results: Vec<Result<(Probabilities, AnsweredBy), Failure>> = questions
            .iter()
            .map(|_| Err(Failure::Unavailable))
            .collect();
        let mut open: Vec<usize> = (0..questions.len()).collect();
        for model in &self.models {
            if open.is_empty() {
                break;
            }
            let asked: Vec<&Question> = open
                .iter()
                .filter_map(|index| questions.get(*index).copied())
                .collect();
            let replies = model.format.answer_all(&asked, evidence);
            let by = AnsweredBy {
                model: model.name.clone(),
                placeholder: model.format.is_placeholder(),
            };
            for (index, reply) in open.iter().zip(replies) {
                if let Some(slot) = results.get_mut(*index) {
                    *slot = reply.map(|p| (p, by.clone()));
                }
            }
            open.retain(
                |index| matches!(results.get(*index), Some(Err(failure)) if failure.falls_back()),
            );
        }
        results
    }
}

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
