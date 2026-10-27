//! TypeSafe's Jev defaults over the shared System One HTTP transport.

use std::path::PathBuf;
use std::time::Duration;

use chauffeur_core::{Answer, Question, SystemOne, SystemOneError};
pub use chauffeur_judge_http::DEFAULT_TIMEOUT;
use chauffeur_judge_http::{HttpClient, HttpConfig};

pub const DEFAULT_BASE_URL: &str = "https://api.typesafe.ai";
pub const DEFAULT_MODEL: &str = "jev-latest";

#[derive(Clone)]
pub struct JevConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout: Duration,
    pub record: Option<PathBuf>,
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
            record: std::env::var_os("CHAUFFEUR_RECORD_JUDGMENTS")
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
        })
    }
}

pub struct JevClient(HttpClient);

impl JevClient {
    /// Must be built outside an async runtime.
    pub fn new(config: JevConfig) -> Result<Self, String> {
        HttpClient::new(
            "jev",
            HttpConfig {
                base_url: config.base_url,
                api_key: config.api_key,
                model: config.model,
                timeout: config.timeout,
                record: config.record,
            },
        )
        .map(Self)
    }
}

impl SystemOne for JevClient {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn ask(&mut self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        self.0.ask(state, questions)
    }

    fn ask_by(
        &mut self,
        state: &str,
        questions: &[Question],
        deadline: std::time::Instant,
    ) -> Result<Vec<Answer>, SystemOneError> {
        self.0.ask_by(state, questions, deadline)
    }
}
