use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chauffeur_core::Failure;
use serde::Deserialize;

use crate::confidence::Probabilities;
use crate::config::{ConfigError, PlaceholderEntry};
use crate::format::{Evidence, Format};
use crate::question::Question;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswersFile {
    answers: Vec<RecordedFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordedFile {
    evidence: BTreeMap<String, String>,
    p: Option<BTreeMap<String, f64>>,
    failed: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum Reply {
    P(BTreeMap<String, f64>),
    Failed(Failure),
}

#[derive(Clone, Debug, PartialEq)]
struct Recorded {
    evidence: BTreeMap<String, String>,
    reply: Reply,
}

impl Recorded {
    /// Whether the question sees every block this entry lists, with the same content.
    fn matches(&self, question: &Question, evidence: &Evidence) -> bool {
        self.evidence.iter().all(|(name, content)| {
            question.evidence().contains(name) && evidence.get(name) == Some(content.as_str())
        })
    }
}

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
            Some(path) => read_answers(path)?,
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

fn read_answers(path: &Path) -> Result<Vec<Recorded>, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })?;
    let file: AnswersFile = serde_json::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.to_owned(),
        source,
    })?;
    file.answers
        .into_iter()
        .enumerate()
        .map(|(index, recorded)| {
            let reply = match (recorded.p, recorded.failed) {
                (Some(p), None) => Reply::P(p),
                (None, Some(name)) => {
                    Reply::Failed(Failure::from_name(&name).ok_or_else(|| {
                        placeholder_error(path, index, &format!("`{name}` is not a failure"))
                    })?)
                }
                _ => {
                    return Err(placeholder_error(
                        path,
                        index,
                        "needs exactly one of `p` and `failed`",
                    ));
                }
            };
            Ok(Recorded {
                evidence: recorded.evidence,
                reply,
            })
        })
        .collect()
}

fn placeholder_error(path: &Path, index: usize, reason: &str) -> ConfigError {
    ConfigError::Placeholder {
        path: PathBuf::from(path),
        index,
        reason: reason.to_owned(),
    }
}
