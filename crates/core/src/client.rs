//! Typed HTTP client for a Chauffeur daemon.

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::effect::Effect;
use crate::protocol::{
    DaemonRequest, DaemonResponse, METHOD_HEALTH, METHOD_SIGNAL, METHOD_SKILLS, SignalParams,
    SignalResult, SkillsResult,
};
use crate::signal::Signal;

pub const DEFAULT_DAEMON_URL: &str = "http://127.0.0.1:18790";

#[derive(Clone)]
pub struct DaemonClient {
    base_url: String,
    token: Option<String>,
    http: reqwest::Client,
}

impl DaemonClient {
    pub fn new(base_url: impl Into<String>, token: Option<String>) -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| format!("build HTTP client: {error}"))?;

        Ok(Self {
            base_url: base_url.into().trim_end_matches('/').to_string(),
            token,
            http,
        })
    }

    pub async fn health(&self) -> Result<(), String> {
        let _: Value = self.call(METHOD_HEALTH, &()).await?;
        Ok(())
    }

    /// Parsed remote skills and last-refresh status, served by the shared source worker.
    pub async fn skills(&self) -> Result<SkillsResult, String> {
        self.call(METHOD_SKILLS, &()).await
    }

    pub async fn signal(&self, signal: Signal) -> Result<Vec<Effect>, String> {
        self.signal_with_deadline(signal, None).await
    }

    /// Report a signal with the host's remaining milliseconds, when bounded.
    pub async fn signal_with_deadline(
        &self,
        signal: Signal,
        deadline_ms: Option<u64>,
    ) -> Result<Vec<Effect>, String> {
        let mut params = SignalParams {
            signal,
            deadline_ms,
        };
        let result: Result<SignalResult, String> = self.call(METHOD_SIGNAL, &params).await;
        let result = match result {
            Err(error)
                if deadline_ms.is_some() && error.contains("unknown field `deadline_ms`") =>
            {
                params.deadline_ms = None;
                self.call(METHOD_SIGNAL, &params).await?
            }
            result => result?,
        };

        Ok(result.effects)
    }

    async fn call<P, R>(&self, method: &str, params: &P) -> Result<R, String>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let params = serde_json::to_value(params).map_err(|error| error.to_string())?;
        let body = DaemonRequest {
            id: Some(1),
            method: method.to_string(),
            params,
        };
        let mut request = self.http.post(format!("{}/rpc", self.base_url)).json(&body);

        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }

        let response = request
            .send()
            .await
            .map_err(|error| format!("call daemon: {error}"))?;
        let status = response.status();
        let envelope: DaemonResponse = response
            .json()
            .await
            .map_err(|error| format!("decode daemon response: {error}"))?;

        if let Some(error) = envelope.error {
            return Err(error);
        }

        if !status.is_success() {
            return Err(format!("daemon returned HTTP {status}"));
        }

        let value = envelope.result.ok_or("daemon response has no result")?;
        serde_json::from_value(value).map_err(|error| format!("decode daemon result: {error}"))
    }
}
