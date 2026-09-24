//! What the engine did with one signal, for the audit log. Text is redacted
//! and clipped here, so a trace is safe to write to disk.

use serde::Serialize;

use crate::redact::redact_secrets;
use crate::system_one::{Answer, AnswerValue, Question, QuestionKind};

const MAX_INSTRUCTION_CHARS: usize = 600;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Trace {
    /// The irreversible-harm pattern that vetoed the signal, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub veto: Option<String>,
    /// Every question sent to System One, namespaced `capability/question`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<TracedQuestion>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<TracedAnswer>,
    /// Why System One failed, when it did; capabilities then applied their
    /// failure posture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Time spent waiting for System One.
    pub elapsed_ms: u64,
    /// What the backstop and redactor learned from this signal's answers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub learned: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TracedQuestion {
    pub round: usize,
    pub id: String,
    pub kind: &'static str,
    pub instructions: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TracedAnswer {
    pub round: usize,
    pub id: String,
    pub value: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl TracedQuestion {
    #[must_use]
    pub fn new(question: &Question, round: usize) -> Self {
        let (kind, options) = match &question.kind {
            QuestionKind::Choice { options } => (
                "choice",
                options.iter().map(|option| option.value.clone()).collect(),
            ),
            QuestionKind::Score { levels } => ("score", levels.clone()),
            QuestionKind::Noul => ("noul", Vec::new()),
        };

        Self {
            round,
            id: question.id.clone(),
            kind,
            instructions: redact_secrets(&question.instructions)
                .chars()
                .take(MAX_INSTRUCTION_CHARS)
                .collect(),
            options,
        }
    }
}

impl TracedAnswer {
    #[must_use]
    pub fn new(answer: &Answer, round: usize) -> Self {
        let value = match &answer.value {
            AnswerValue::Choice(choice) => serde_json::json!(choice),
            AnswerValue::Score(score) => serde_json::json!(score),
            AnswerValue::Noul(probability) => serde_json::json!(probability),
        };

        Self {
            round,
            id: answer.id.clone(),
            value,
            confidence: answer.confidence,
        }
    }
}
