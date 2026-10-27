use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chauffeur_core::Failure;
use serde::Deserialize;

use crate::config::ConfigError;
use crate::format::Evidence;
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

/// What a recorded answer gives: probabilities, or a failure.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Reply {
    P(BTreeMap<String, f64>),
    Failed(Failure),
}

/// One recorded answer and the evidence it was given for.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Recorded {
    evidence: BTreeMap<String, String>,
    pub(super) reply: Reply,
}

impl Recorded {
    /// Whether the question sees every block this entry lists, with the same content.
    pub(super) fn matches(&self, question: &Question, evidence: &Evidence) -> bool {
        self.evidence.iter().all(|(name, content)| {
            question.evidence().contains(name) && evidence.get(name) == Some(content.as_str())
        })
    }
}

/// The recorded answers in the file at `path`, in order.
pub(super) fn read(path: &Path) -> Result<Vec<Recorded>, ConfigError> {
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
                        malformed(path, index, &format!("`{name}` is not a failure"))
                    })?)
                }
                _ => {
                    return Err(malformed(
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

fn malformed(path: &Path, index: usize, reason: &str) -> ConfigError {
    ConfigError::Placeholder {
        path: PathBuf::from(path),
        index,
        reason: reason.to_owned(),
    }
}
