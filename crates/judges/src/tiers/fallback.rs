use chauffeur_core::Failure;

use super::{Tier, system_one};
use crate::confidence::{AnsweredBy, Probabilities};
use crate::config::{ConfigError, ModelEntry, Models};
use crate::format::{Evidence, Format};
use crate::placeholder::Placeholder;
use crate::question::Question;

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
                    match key(&entry.key.env).and_then(|key| system_one::format(entry, key)) {
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
