use std::time::Duration;

use chauffeur_core::Failure;
use serde_json::{Map, Value, json};

use super::SystemOneEntry;
use super::wire::{Response, is_noul, read_noul, state};
use crate::confidence::Probabilities;
use crate::format::{Evidence, Format};
use crate::question::Question;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_RESPONSE_BYTES: u64 = 262_144;

/// The `/v1/systemone` API that Jev speaks. It asks yes/no questions with
/// `noul`; any other question fails as invalid.
pub struct SystemOne {
    agent: ureq::Agent,
    url: String,
    model: String,
    key: String,
}

impl SystemOne {
    /// A client for `entry`, sending `key` as its bearer token.
    #[must_use]
    pub fn new(entry: &SystemOneEntry, key: String) -> Self {
        let timeout = entry
            .timeout_ms
            .map_or(DEFAULT_TIMEOUT, Duration::from_millis);
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .into();
        let url = format!("{}/v1/systemone", entry.endpoint.trim_end_matches('/'));
        Self {
            agent,
            url,
            model: entry.model.clone(),
            key,
        }
    }

    fn call(&self, body: &Value) -> Result<Response, Failure> {
        let mut response = self
            .agent
            .post(&self.url)
            .header("authorization", format!("Bearer {}", self.key))
            .send_json(body)
            .map_err(|error| failure(&error))?;
        match response.status().as_u16() {
            200..=299 => {}
            429 | 529 => return Err(Failure::RateLimited),
            408 => return Err(Failure::Timeout),
            _ => return Err(Failure::Unavailable),
        }
        let text = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_string()
            .map_err(|error| failure(&error))?;
        serde_json::from_str(&text).map_err(|_| Failure::Invalid)
    }
}

/// A transport error as a failure. Its text is dropped: it can carry the
/// request's context.
const fn failure(error: &ureq::Error) -> Failure {
    match error {
        ureq::Error::Timeout(_) => Failure::Timeout,
        ureq::Error::BodyExceedsLimit(_) => Failure::Invalid,
        _ => Failure::Unavailable,
    }
}

impl Format for SystemOne {
    fn answer_all(
        &self,
        questions: &[&Question],
        evidence: &Evidence,
    ) -> Vec<Result<Probabilities, Failure>> {
        let ids: Vec<String> = (0..questions.len())
            .map(|index| format!("q{index}"))
            .collect();
        let mut asked = Map::new();
        for (id, question) in ids.iter().zip(questions) {
            if is_noul(question) {
                asked.insert(
                    id.clone(),
                    json!({ "type": "noul", "instructions": question.instructions() }),
                );
            }
        }
        if asked.is_empty() {
            return questions.iter().map(|_| Err(Failure::Invalid)).collect();
        }
        let body =
            json!({ "model": self.model, "state": state(questions, evidence), "questions": asked });
        match self.call(&body) {
            Ok(response) => ids
                .iter()
                .zip(questions)
                .map(|(id, question)| {
                    if is_noul(question) {
                        read_noul(question, response.answers.get(id))
                    } else {
                        Err(Failure::Invalid)
                    }
                })
                .collect(),
            Err(failure) => questions.iter().map(|_| Err(failure)).collect(),
        }
    }
}
