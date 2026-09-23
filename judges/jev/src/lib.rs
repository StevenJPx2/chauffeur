//! System One provider for TypeSafe AI's hosted Jev model.
//!
//! State is secret-redacted before it leaves the machine. Jev evaluates every
//! question against the state in one shared pass, so one request serves a
//! whole routing pass.

use std::collections::HashMap;
use std::time::Duration;

use chauffeur_core::{
    Answer, AnswerValue, Question, QuestionKind, SystemOne, SystemOneError, redact_secrets,
    validate_answers,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
pub const DEFAULT_MODEL: &str = "jev-latest";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: usize = 262_144;

pub struct JevConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout: Duration,
}

impl JevConfig {
    /// `TYPESAFE_API_KEY` is required; returns `None` when it is unset.
    #[must_use]
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("TYPESAFE_API_KEY")
            .ok()
            .filter(|key| !key.is_empty())?;

        Some(Self {
            base_url: std::env::var("TYPESAFE_BASE_URL")
                .unwrap_or_else(|_| DEFAULT_BASE_URL.into()),
            api_key,
            model: std::env::var("TYPESAFE_DEFAULT_MODEL").unwrap_or_else(|_| DEFAULT_MODEL.into()),
            timeout: DEFAULT_TIMEOUT,
        })
    }
}

pub struct JevClient {
    http: reqwest::blocking::Client,
    endpoint: String,
    api_key: String,
    model: String,
}

impl JevClient {
    /// Build the blocking HTTP client. Must not run inside an async runtime.
    pub fn new(config: JevConfig) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| format!("build Jev HTTP client: {error}"))?;

        Ok(Self {
            http,
            endpoint: format!("{}/v1/systemone", config.base_url.trim_end_matches('/')),
            api_key: config.api_key,
            model: config.model,
        })
    }
}

impl SystemOne for JevClient {
    fn name(&self) -> &str {
        "jev"
    }

    fn ask(&mut self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        let body = request_body(&self.model, &redact_secrets(state), questions);
        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .map_err(|error| SystemOneError(format!("Jev request: {error}")))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .map_err(|error| SystemOneError(format!("Jev response: {error}")))?;

        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(SystemOneError(format!(
                "Jev response exceeds {MAX_RESPONSE_BYTES} bytes"
            )));
        }
        if !status.is_success() {
            return Err(SystemOneError(format!("Jev returned HTTP {status}")));
        }

        let answers = parse_answers(&bytes)?;

        validate_answers(questions, &answers)?;

        Ok(answers)
    }
}

fn request_body(model: &str, state: &str, questions: &[Question]) -> Value {
    let questions: Map<String, Value> = questions
        .iter()
        .map(|question| (question.id.clone(), question_json(question)))
        .collect();

    json!({ "model": model, "state": state, "questions": questions })
}

fn question_json(question: &Question) -> Value {
    match &question.kind {
        QuestionKind::Choice { options } => {
            let criteria: Map<String, Value> = options
                .iter()
                .map(|option| {
                    (
                        option.value.clone(),
                        Value::String(option.description.clone()),
                    )
                })
                .collect();

            json!({ "type": "choice", "instructions": question.instructions, "criteria": criteria })
        }
        QuestionKind::Score { levels } => {
            json!({ "type": "score", "instructions": question.instructions, "criteria": levels })
        }
        QuestionKind::Noul => json!({ "type": "noul", "instructions": question.instructions }),
    }
}

#[derive(Deserialize)]
struct Response {
    answers: HashMap<String, WireAnswer>,
}

#[derive(Deserialize)]
struct WireAnswer {
    #[serde(rename = "type")]
    kind: String,
    choice: Option<String>,
    score: Option<f32>,
    noul: Option<f32>,
    confidence: Option<f32>,
}

fn parse_answers(bytes: &[u8]) -> Result<Vec<Answer>, SystemOneError> {
    let response: Response = serde_json::from_slice(bytes)
        .map_err(|error| SystemOneError(format!("decode Jev response: {error}")))?;

    response
        .answers
        .into_iter()
        .map(|(id, wire)| {
            let value = match (wire.kind.as_str(), wire.choice, wire.score, wire.noul) {
                ("choice", Some(choice), None, None) => AnswerValue::Choice(choice),
                ("score", None, Some(score), None) => AnswerValue::Score(score),
                ("noul", None, None, Some(probability)) => AnswerValue::Noul(probability),
                (kind, ..) => {
                    return Err(SystemOneError(format!(
                        "Jev answer {id}: malformed {kind} value"
                    )));
                }
            };

            Ok(Answer {
                id,
                value,
                confidence: wire.confidence,
            })
        })
        .collect()
}
