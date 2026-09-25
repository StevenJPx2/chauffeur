//! sourcefed's monitors, through its daemon's JSON RPC (`POST /rpc`). Each
//! Chauffeur agent is a sourcefed target: an OpenCode session's monitors
//! belong to `{kind: "opencode-session", id: <session ID>}`, the same session
//! ID Chauffeur uses for the agent.

use std::time::Duration;

use chauffeur_capability_monitors::{Monitor, Monitors, Watch};
use serde::Deserialize;
use serde_json::{Value, json};

pub const DEFAULT_URL: &str = "http://127.0.0.1:18787";
pub const DEFAULT_TARGET_KIND: &str = "opencode-session";
/// sourcefed is local; a slow answer is treated as unavailable.
const TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: usize = 262_144;

pub struct SourcefedConfig {
    pub url: String,
    pub token: Option<String>,
    pub target_kind: String,
}

impl SourcefedConfig {
    /// `SOURCEFED_DAEMON_URL` and `SOURCEFED_DAEMON_TOKEN`, as sourcefed reads
    /// them, and `CHAUFFEUR_SOURCEFED_TARGET_KIND` for the host's target kind.
    #[must_use]
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());

        Self {
            url: var("SOURCEFED_DAEMON_URL").unwrap_or_else(|| DEFAULT_URL.into()),
            token: var("SOURCEFED_DAEMON_TOKEN"),
            target_kind: var("CHAUFFEUR_SOURCEFED_TARGET_KIND")
                .unwrap_or_else(|| DEFAULT_TARGET_KIND.into()),
        }
    }
}

pub struct Sourcefed {
    http: reqwest::blocking::Client,
    endpoint: String,
    token: Option<String>,
    target_kind: String,
}

impl Sourcefed {
    /// Build the blocking client. Must not run inside an async runtime.
    pub fn new(config: SourcefedConfig) -> Result<Self, String> {
        let http = reqwest::blocking::Client::builder()
            .timeout(TIMEOUT)
            .build()
            .map_err(|error| format!("build sourcefed HTTP client: {error}"))?;

        Ok(Self {
            http,
            endpoint: format!("{}/rpc", config.url.trim_end_matches('/')),
            token: config.token,
            target_kind: config.target_kind,
        })
    }

    fn request(&self, method: &str, agent_id: &str, mut params: Value) -> Result<Value, String> {
        params["target"] = json!({ "kind": self.target_kind, "id": agent_id });

        let mut request = self
            .http
            .post(&self.endpoint)
            .json(&json!({ "id": 1, "method": method, "params": params }));

        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }

        let response = request
            .send()
            .map_err(|error| format!("sourcefed {method}: {error}"))?;
        let bytes = response
            .bytes()
            .map_err(|error| format!("sourcefed {method}: {error}"))?;

        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err(format!(
                "sourcefed {method}: response exceeds {MAX_RESPONSE_BYTES} bytes"
            ));
        }

        let reply: Reply = serde_json::from_slice(&bytes)
            .map_err(|error| format!("sourcefed {method}: {error}"))?;

        let result = match (reply.result, reply.error) {
            (_, Some(error)) => return Err(format!("sourcefed {method}: {error}")),
            (Some(result), None) => result,
            (None, None) => return Err(format!("sourcefed {method}: no result")),
        };

        // A refused operation is still an HTTP success: `{ok: false, error}`.
        if result["ok"] == json!(false) {
            let error = result["error"].as_str().unwrap_or("refused");

            return Err(format!("sourcefed {method}: {error}"));
        }

        Ok(result)
    }
}

#[derive(Deserialize)]
struct Reply {
    result: Option<Value>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct Record {
    id: String,
    source: Value,
    enabled: bool,
}

impl From<Record> for Monitor {
    fn from(record: Record) -> Self {
        Self {
            id: record.id,
            watch: watch(&record.source),
            enabled: record.enabled,
        }
    }
}

/// The watch a sourcefed source record describes, when Chauffeur models it.
fn watch(source: &Value) -> Option<Watch> {
    let text = |field: &str| source[field].as_str().map(str::to_string);

    match source["type"].as_str()? {
        "github" => Some(Watch::GithubPr {
            repo: text("repo")?,
            number: source["prNumber"].as_u64()?,
        }),
        "jira" => Some(Watch::JiraIssue {
            key: text("issueKey")?,
        }),
        "slack" => Some(Watch::SlackThread {
            channel: text("channelId")?,
            thread: text("threadTs")?,
        }),
        _ => None,
    }
}

/// `monitor.create` parameters for a watch.
fn create_params(watch: &Watch) -> Value {
    match watch {
        Watch::GithubPr { repo, number } => {
            json!({ "name": watch.name(), "sourceType": "github", "repo": repo, "prNumber": number })
        }
        Watch::JiraIssue { key } => {
            json!({ "name": watch.name(), "sourceType": "jira", "issueKey": key })
        }
        Watch::SlackThread { channel, thread } => {
            json!({ "name": watch.name(), "sourceType": "slack", "channelId": channel, "threadTs": thread })
        }
    }
}

impl Monitors for Sourcefed {
    fn list(&self, agent_id: &str) -> Result<Vec<Monitor>, String> {
        let result = self.request("monitor.list", agent_id, json!({}))?;
        let records: Vec<Record> = serde_json::from_value(result["monitors"].clone())
            .map_err(|error| format!("sourcefed monitor.list: {error}"))?;

        Ok(records.into_iter().map(Monitor::from).collect())
    }

    fn create(&self, agent_id: &str, watch: &Watch) -> Result<Monitor, String> {
        let result = self.request("monitor.create", agent_id, create_params(watch))?;
        let record: Record = serde_json::from_value(result["monitor"].clone())
            .map_err(|error| format!("sourcefed monitor.create: {error}"))?;

        Ok(record.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_records_map_to_watches_and_back() {
        let pr = Watch::GithubPr {
            repo: "acme/app".into(),
            number: 42,
        };
        let issue = Watch::JiraIssue {
            key: "ADEPT-1".into(),
        };
        let thread = Watch::SlackThread {
            channel: "C1".into(),
            thread: "1700000000.000100".into(),
        };

        for watch in [&pr, &issue, &thread] {
            let params = create_params(watch);
            let source = json!({
                "type": params["sourceType"],
                "repo": params["repo"],
                "prNumber": params["prNumber"],
                "issueKey": params["issueKey"],
                "channelId": params["channelId"],
                "threadTs": params["threadTs"],
            });

            assert_eq!(super::watch(&source).as_ref(), Some(watch));
        }

        assert_eq!(
            super::watch(&json!({ "type": "slack", "channelId": "D1" })),
            None
        );
    }
}
