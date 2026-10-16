//! What the engine did with one signal, for the audit log. Text is redacted
//! and clipped here, so a trace is safe to write to disk.

use serde::Serialize;

use crate::redact::redact_secrets;
use crate::system_one::{Answer, AnswerValue, Question, QuestionKind};
use crate::system_two::{AttemptMetadata, Reasoned, Reasoning};

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
    /// Every System Two reasoning, with its verdict or why there is none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reasonings: Vec<TracedReasoning>,
    /// Why System One failed, when it did; capabilities then applied their
    /// failure posture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Time spent waiting for inference rounds.
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TracedReasoning {
    pub round: usize,
    pub id: String,
    pub profile: String,
    #[serde(flatten)]
    pub metadata: AttemptMetadata,
    pub instructions: String,
    /// The IDs of the evidence it was given.
    pub evidence: Vec<String>,
    /// The verdict as JSON, clipped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verdict: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

impl TracedReasoning {
    /// Attach execution facts with the same clipping and secret redaction as
    /// other provider-controlled trace text.
    #[must_use]
    pub fn with_metadata(mut self, mut metadata: AttemptMetadata) -> Self {
        for text in [
            &mut metadata.provider,
            &mut metadata.backend,
            &mut metadata.requested_model,
            &mut metadata.completion.reported_model,
        ]
        .into_iter()
        .flatten()
        {
            *text = redact_secrets(text)
                .chars()
                .take(MAX_INSTRUCTION_CHARS)
                .collect();
        }
        self.metadata = metadata;
        self
    }

    #[must_use]
    pub fn new(request: &Reasoning, outcome: &Reasoned, round: usize) -> Self {
        let clip = |text: &str| -> String {
            redact_secrets(text)
                .chars()
                .take(MAX_INSTRUCTION_CHARS)
                .collect()
        };
        let (verdict, failure) = match outcome {
            Ok(verdict) => (Some(clip(&verdict.to_string())), None),
            Err(failure) => (None, Some(clip(&failure.to_string()))),
        };

        Self {
            round,
            id: request.id.clone(),
            profile: request.profile.clone(),
            metadata: AttemptMetadata::default(),
            instructions: clip(&request.instructions),
            evidence: request
                .evidence
                .iter()
                .map(|piece| piece.id.clone())
                .collect(),
            verdict,
            failure,
        }
    }
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
