//! Shared `/v1/systemone` transport for hosted decision judges.
//!
//! The engine redacts state and questions before calling a judge. Providers
//! supply only their name, endpoint, credentials and model defaults.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chauffeur_core::{
    Answer, AnswerValue, Question, QuestionKind, SystemOne, SystemOneError, validate_answers,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: usize = 262_144;

#[derive(Clone)]
pub struct HttpConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout: Duration,
    /// Append every request and reply to this JSONL file, for replaying the
    /// same judgments against other System One models.
    pub record: Option<PathBuf>,
}

pub struct HttpClient {
    glide: bool,
    name: &'static str,
    http: reqwest::blocking::Client,
    endpoint: String,
    api_key: String,
    model: String,
    record: Option<PathBuf>,
}

impl HttpClient {
    /// Build the blocking HTTP client. Must not run inside an async runtime.
    pub fn new(name: &'static str, config: HttpConfig) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|error| format!("build {name} HTTP client: {error}"))?;

        Ok(Self {
            glide: false,
            name,
            http,
            endpoint: format!("{}/v1/systemone", config.base_url.trim_end_matches('/')),
            api_key: config.api_key,
            model: config.model,
            record: config.record,
        })
    }

    /// Fastino's System One dialect: no provider storage and weighted Score
    /// values read from `expected_level`, never the winning index `score`.
    pub fn new_glide(config: HttpConfig) -> Result<Self, String> {
        let mut client = Self::new("glide", config)?;
        client.glide = true;
        Ok(client)
    }

    fn call(&self, body: &Value, timeout: Option<Duration>) -> Result<Vec<Answer>, SystemOneError> {
        let mut request = self.http.post(&self.endpoint);
        if let Some(timeout) = timeout {
            request = request.timeout(timeout);
        }
        let response = request
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .map_err(|error| SystemOneError(format!("{} request: {error}", self.name)))?;
        let status = response.status();
        let mut bytes = Vec::new();
        response
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| SystemOneError(format!("{} response: {error}", self.name)))?;

        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(SystemOneError(format!(
                "{} response exceeds {MAX_RESPONSE_BYTES} bytes",
                self.name
            )));
        }
        if !status.is_success() {
            return Err(SystemOneError(format!(
                "{} returned HTTP {status}{}",
                self.name,
                error_type(&bytes)
            )));
        }

        parse_answers(self.name, &bytes, self.glide)
    }

    /// Best effort: a recording that cannot be written never fails a judgment.
    fn record(
        &self,
        body: &Value,
        result: &Result<Vec<Answer>, SystemOneError>,
        elapsed: Duration,
    ) {
        let Some(path) = &self.record else { return };
        let outcome = match result {
            Ok(answers) => json!({ "answers": answers_json(answers) }),
            Err(error) => json!({ "error": error.0 }),
        };
        let line = json!({
            "provider": self.name,
            "at_ms": SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_millis()),
            "elapsed_ms": elapsed.as_millis(),
            "request": body,
            "reply": outcome,
        });
        let written = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut file| file.write_all(format!("{line}\n").as_bytes()));

        if let Err(error) = written {
            eprintln!("chauffeur: record judgment to {}: {error}", path.display());
        }
    }
}

/// The provider's own reason, when it names one this file knows: `: 400 Bad
/// Request: max_tokens_exceeded`. The list is an allowlist read off a live
/// API, because a reply is never echoed otherwise: its text carries the
/// context that was judged. Add a name here once a provider is seen to send it.
fn error_type(bytes: &[u8]) -> String {
    const KNOWN: &[&str] = &["max_tokens_exceeded", "context_length_exceeded"];

    let body = serde_json::from_slice::<Value>(bytes).ok();
    let named = body
        .as_ref()
        .and_then(|body| {
            body.get("detail")
                .and_then(|detail| detail.get("error_type"))
                .or_else(|| body.get("error_type"))
                .and_then(Value::as_str)
        })
        .filter(|reason| KNOWN.contains(reason));

    named.map_or_else(String::new, |reason| format!(": {reason}"))
}

fn answers_json(answers: &[Answer]) -> Value {
    let answers: Map<String, Value> = answers
        .iter()
        .map(|answer| {
            let mut wire = match &answer.value {
                AnswerValue::Choice(choice) => json!({ "type": "choice", "choice": choice }),
                AnswerValue::Score(score) => json!({ "type": "score", "score": score }),
                AnswerValue::Noul(probability) => json!({ "type": "noul", "noul": probability }),
            };
            if let Some(confidence) = answer.confidence {
                wire["confidence"] = json!(confidence);
            }
            (answer.id.clone(), wire)
        })
        .collect();

    Value::Object(answers)
}

impl SystemOne for HttpClient {
    fn name(&self) -> &str {
        self.name
    }

    fn ask(&mut self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        self.ask_within(state, questions, None)
    }

    fn ask_by(
        &mut self,
        state: &str,
        questions: &[Question],
        deadline: Instant,
    ) -> Result<Vec<Answer>, SystemOneError> {
        let remaining = deadline.saturating_duration_since(Instant::now());

        if remaining.is_zero() {
            return Err(SystemOneError(format!("{} deadline passed", self.name)));
        }
        self.ask_within(state, questions, Some(remaining))
    }
}

impl HttpClient {
    /// The client's own timeout still applies; `timeout` can only shorten it.
    fn ask_within(
        &self,
        state: &str,
        questions: &[Question],
        timeout: Option<Duration>,
    ) -> Result<Vec<Answer>, SystemOneError> {
        let mut body = request_body(&self.model, state, questions);
        if self.glide {
            body["store"] = json!(false);
        }
        let started = Instant::now();
        let result = self.call(&body, timeout).and_then(|answers| {
            validate_answers(questions, &answers)?;
            Ok(answers)
        });

        self.record(&body, &result, started.elapsed());

        result
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
    expected_level: Option<f32>,
    noul: Option<f32>,
    confidence: Option<f32>,
}

fn parse_answers(name: &str, bytes: &[u8], glide: bool) -> Result<Vec<Answer>, SystemOneError> {
    let response: Response = serde_json::from_slice(bytes)
        .map_err(|error| SystemOneError(format!("decode {name} response: {error}")))?;

    response
        .answers
        .into_iter()
        .map(|(id, wire)| {
            let score = if glide && wire.kind == "score" {
                wire.expected_level
            } else {
                wire.score
            };
            let value = match (wire.kind.as_str(), wire.choice, score, wire.noul) {
                ("choice", Some(choice), None, None) => AnswerValue::Choice(choice),
                ("score", None, Some(score), None) => AnswerValue::Score(score),
                ("noul", None, None, Some(probability)) => AnswerValue::Noul(probability),
                (kind, ..) => {
                    return Err(SystemOneError(format!(
                        "{name} answer {id}: malformed {kind} value"
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
