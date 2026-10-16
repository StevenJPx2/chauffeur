//! A System Two backend that reasons with OpenAI models over your ChatGPT
//! subscription, through the Codex CLI's login and Responses endpoint. The
//! output is held to the request's JSON Schema in strict mode.

mod auth;

use std::io::{BufRead, BufReader, Read};
use std::sync::Arc;

use chauffeur_core::system_two::MAX_VERDICT_BYTES;
use chauffeur_core::{Backend, Completion, CompletionMetadata, Reasoned, ReasoningFailure, Usage};
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use serde_json::{Value, json};

pub use auth::{CodexAuth, TOKEN_URL, Token};

pub const RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
/// Bytes of event stream read at most.
const MAX_STREAM_BYTES: u64 = 4 * 1024 * 1024;
/// Characters of a provider's error message kept.
const MAX_ERROR_CHARS: usize = 300;

pub struct CodexBackend {
    auth: Arc<CodexAuth>,
    http: Client,
    endpoint: String,
}

impl CodexBackend {
    /// A backend using `auth`. Build it off any async runtime: its client
    /// blocks.
    ///
    /// # Errors
    ///
    /// The HTTP client could not be built.
    pub fn new(auth: Arc<CodexAuth>) -> Result<Self, String> {
        let http = Client::builder()
            .build()
            .map_err(|error| format!("build the Codex HTTP client: {error}"))?;

        Ok(Self {
            auth,
            http,
            endpoint: RESPONSES_URL.into(),
        })
    }

    /// Send requests to `url`, such as a test server.
    #[must_use]
    pub fn with_endpoint(mut self, url: impl Into<String>) -> Self {
        self.endpoint = url.into();
        self
    }

    fn send(&self, completion: &Completion<'_>) -> Result<Response, ReasoningFailure> {
        let started = std::time::Instant::now();
        let token = self
            .auth
            .token(&self.http)
            .map_err(ReasoningFailure::Unavailable)?;
        let timeout = completion.timeout.saturating_sub(started.elapsed());
        if timeout < chauffeur_core::system_two::MIN_REASONING_TIME {
            return Err(ReasoningFailure::TimedOut);
        }
        let mut request = self
            .http
            .post(&self.endpoint)
            .timeout(timeout)
            .bearer_auth(&token.access)
            .header("originator", "codex_cli_rs")
            .header("openai-beta", "responses=experimental")
            .header("accept", "text/event-stream")
            .json(&body(completion));

        if !token.account.is_empty() {
            request = request.header("chatgpt-account-id", &token.account);
        }

        let response = request.send().map_err(transport)?;
        let status = response.status();

        if status.is_success() {
            return Ok(response);
        }
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            self.auth.forget();
        }

        let detail = clip(&response.text().unwrap_or_default());

        Err(ReasoningFailure::Failed(
            if status == StatusCode::TOO_MANY_REQUESTS {
                format!("rate limited (HTTP 429): {detail}")
            } else {
                format!("HTTP {status}: {detail}")
            },
        ))
    }
}

impl Backend for CodexBackend {
    fn complete(
        &mut self,
        completion: &Completion<'_>,
        metadata: &mut CompletionMetadata,
    ) -> Reasoned {
        let response = self.send(completion)?;
        let text = read_stream(response, metadata)?;

        serde_json::from_str(&text)
            .map_err(|error| ReasoningFailure::Invalid(format!("output is not JSON: {error}")))
    }
}

/// The Responses request: trusted instructions apart from the input, the
/// schema enforced strictly, nothing stored.
fn body(completion: &Completion<'_>) -> Value {
    let mut body = json!({
        "model": completion.model,
        "instructions": completion.instructions,
        "input": [{
            "role": "user",
            "content": [{ "type": "input_text", "text": completion.input }],
        }],
        "text": { "format": {
            "type": "json_schema",
            "name": format_name(completion.name),
            "schema": strict(completion.schema.clone()),
            "strict": true,
        }},
        "max_output_tokens": completion.max_output_tokens,
        "store": false,
        "stream": true,
    });

    if let Some(effort) = completion.effort {
        body["reasoning"] = json!({ "effort": effort });
    }

    body
}

/// A format name the API accepts: letters, digits, `_` and `-`, at most 64.
fn format_name(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();

    if name.is_empty() {
        "verdict".into()
    } else {
        name
    }
}

/// The schema as strict mode accepts it: an empty `enum`, such as evidence
/// IDs for a request with no evidence, becomes any string, which the host's
/// own validation then rejects.
fn strict(mut schema: Value) -> Value {
    match &mut schema {
        Value::Object(object) => {
            if object
                .get("enum")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
            {
                object.remove("enum");
            }
            for value in object.values_mut() {
                *value = strict(value.take());
            }
        }
        Value::Array(items) => {
            for item in items {
                *item = strict(item.take());
            }
        }
        _ => {}
    }

    schema
}

fn transport(error: reqwest::Error) -> ReasoningFailure {
    if error.is_timeout() {
        ReasoningFailure::TimedOut
    } else {
        ReasoningFailure::Failed(format!("request: {error}"))
    }
}

fn clip(text: &str) -> String {
    text.trim().chars().take(MAX_ERROR_CHARS).collect()
}

/// The output text of a streamed response, or why there is none.
fn read_stream(
    response: Response,
    metadata: &mut CompletionMetadata,
) -> Result<String, ReasoningFailure> {
    let reader = BufReader::new(response.take(MAX_STREAM_BYTES));
    let mut text = String::new();
    let mut refused = false;

    for line in reader.lines() {
        let line = line.map_err(|error| match error.kind() {
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                ReasoningFailure::TimedOut
            }
            _ if error.to_string().contains("timed out") => ReasoningFailure::TimedOut,
            _ => ReasoningFailure::Failed(format!("stream: {error}")),
        })?;
        let Some(data) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        let Ok(event) = serde_json::from_str::<Value>(data) else {
            continue;
        };

        record_metadata(&event["response"], metadata);

        match event["type"].as_str().unwrap_or_default() {
            "response.output_text.delta" => {
                text.push_str(event["delta"].as_str().unwrap_or_default());

                if text.len() > MAX_VERDICT_BYTES {
                    return Err(ReasoningFailure::Invalid(format!(
                        "output exceeds {MAX_VERDICT_BYTES} bytes"
                    )));
                }
            }
            "response.refusal.delta" | "response.refusal.done" => refused = true,
            "response.incomplete" => {
                return Err(
                    match event["response"]["incomplete_details"]["reason"].as_str() {
                        Some("max_output_tokens") => ReasoningFailure::Truncated,
                        Some("content_filter") => ReasoningFailure::Refused,
                        reason => ReasoningFailure::Failed(format!(
                            "incomplete: {}",
                            reason.unwrap_or("no reason given")
                        )),
                    },
                );
            }
            "response.failed" | "error" => return Err(provider_error(&event)),
            "response.completed" if refused => return Err(ReasoningFailure::Refused),
            "response.completed" if text.is_empty() => {
                return Err(ReasoningFailure::Invalid("no output text".into()));
            }
            "response.completed" => return Ok(text),
            _ => {}
        }
    }

    Err(if refused {
        ReasoningFailure::Refused
    } else {
        ReasoningFailure::Failed("the stream ended before the response completed".into())
    })
}

fn record_metadata(response: &Value, metadata: &mut CompletionMetadata) {
    if let Some(model) = response["model"].as_str() {
        metadata.reported_model = Some(model.chars().take(MAX_ERROR_CHARS).collect());
    }
    if let Some(usage) = response.get("usage").filter(|usage| usage.is_object()) {
        metadata.usage = Usage {
            input_tokens: usage["input_tokens"].as_u64(),
            output_tokens: usage["output_tokens"].as_u64(),
        };
    }
}

fn provider_error(event: &Value) -> ReasoningFailure {
    let error = [&event["response"]["error"], &event["error"]]
        .into_iter()
        .find(|error| error.is_object())
        .unwrap_or(event);
    let message = clip(
        error["message"]
            .as_str()
            .unwrap_or("the provider returned an error"),
    );
    let code = error["code"].as_str().unwrap_or_default();

    ReasoningFailure::Failed(
        if ["rate_limit", "usage_limit", "quota"]
            .iter()
            .any(|limit| code.contains(limit))
        {
            format!("rate limited ({code}): {message}")
        } else {
            message
        },
    )
}
