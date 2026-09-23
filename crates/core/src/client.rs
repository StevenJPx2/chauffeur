//! Typed HTTP client for a Chauffeur daemon.

use std::time::Duration;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::context::AgentContext;
use crate::protocol::{
    AcknowledgeParams, DaemonRequest, DaemonResponse, METHOD_HEALTH, METHOD_REMINDERS_ACKNOWLEDGE,
    METHOD_REMINDERS_READ, METHOD_SKILL_EVALUATE, METHOD_SKILLS_LIST, METHOD_STEER,
    SkillEvaluateParams, SkillEvaluateResult, SteerParams, SteerResult, TargetParams,
};
use crate::queue::QueuedReminder;
use crate::skill::{SkillContext, SkillResult};
use crate::target::Target;

pub const DEFAULT_DAEMON_URL: &str = "http://127.0.0.1:18788";

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

    pub async fn steer(
        &self,
        target: Target,
        context: AgentContext,
    ) -> Result<SteerResult, String> {
        self.call(METHOD_STEER, &SteerParams { target, context })
            .await
    }

    pub async fn reminders(&self, target: Target) -> Result<Vec<QueuedReminder>, String> {
        self.call(METHOD_REMINDERS_READ, &TargetParams { target })
            .await
    }

    pub async fn acknowledge(
        &self,
        target: Target,
        reminder_ids: Vec<String>,
    ) -> Result<(), String> {
        let _: Value = self
            .call(
                METHOD_REMINDERS_ACKNOWLEDGE,
                &AcknowledgeParams {
                    target,
                    reminder_ids,
                },
            )
            .await?;

        Ok(())
    }

    pub async fn evaluate_skill(
        &self,
        skill_id: impl Into<String>,
        context: SkillContext,
    ) -> Result<SkillResult, String> {
        let result: SkillEvaluateResult = self
            .call(
                METHOD_SKILL_EVALUATE,
                &SkillEvaluateParams {
                    skill_id: skill_id.into(),
                    context,
                },
            )
            .await?;

        Ok(result.evaluation)
    }

    pub async fn skills(&self) -> Result<Vec<String>, String> {
        self.call(METHOD_SKILLS_LIST, &()).await
    }

    pub async fn event_response(&self, target: &Target) -> Result<reqwest::Response, String> {
        let target = serde_json::to_string(target).map_err(|error| error.to_string())?;
        let mut request = self
            .http
            .get(format!("{}/events", self.base_url))
            .query(&[("target", target)]);

        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }

        request
            .send()
            .await
            .map_err(|error| format!("subscribe to daemon: {error}"))
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
