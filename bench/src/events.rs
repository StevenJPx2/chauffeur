//! Metrics from `opencode run --format json` output: one JSON event per line.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_TEXT_CHARS: usize = 500;

/// Token counts summed over every `step_finish`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub reasoning: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

/// What the agent did, from its event stream.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventMetrics {
    /// Parsed events, of any type.
    pub events: u64,
    /// Lines that were not JSON objects.
    pub bad_lines: u64,
    /// `step_finish` events.
    pub steps: u64,
    pub tokens: Tokens,
    pub cost: f64,
    /// Finished tool calls (completed or error).
    pub tool_calls: u64,
    pub tools: BTreeMap<String, u64>,
    pub tool_errors: u64,
    pub error_events: u64,
    pub first_error: Option<String>,
    /// The last `text` part, clipped.
    pub last_text: Option<String>,
}

/// Summarize an events file. Unknown event types are ignored and
/// unparseable lines are counted.
#[must_use]
pub fn parse(text: &str) -> EventMetrics {
    let mut metrics = EventMetrics::default();
    let mut seen_tools = BTreeSet::new();

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(Value::Object(event)) = serde_json::from_str::<Value>(line) else {
            metrics.bad_lines = metrics.bad_lines.saturating_add(1);
            continue;
        };
        metrics.events = metrics.events.saturating_add(1);
        let part = event.get("part").unwrap_or(&Value::Null);

        match event.get("type").and_then(Value::as_str) {
            Some("step_finish") => add_step(&mut metrics, part),
            Some("tool_use") => add_tool(&mut metrics, &mut seen_tools, part),
            Some("text") => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    metrics.last_text = Some(text.chars().take(MAX_TEXT_CHARS).collect());
                }
            }
            Some("error") => {
                metrics.error_events = metrics.error_events.saturating_add(1);
                if metrics.first_error.is_none() {
                    metrics.first_error = Some(error_message(event.get("error")));
                }
            }
            _ => {}
        }
    }

    metrics
}

fn add_step(metrics: &mut EventMetrics, part: &Value) {
    let tokens = part.get("tokens").unwrap_or(&Value::Null);
    let count = |value: &Value, key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    let cache = tokens.get("cache").unwrap_or(&Value::Null);
    let sum = &mut metrics.tokens;

    metrics.steps = metrics.steps.saturating_add(1);
    sum.input = sum.input.saturating_add(count(tokens, "input"));
    sum.output = sum.output.saturating_add(count(tokens, "output"));
    sum.reasoning = sum.reasoning.saturating_add(count(tokens, "reasoning"));
    sum.cache_read = sum.cache_read.saturating_add(count(cache, "read"));
    sum.cache_write = sum.cache_write.saturating_add(count(cache, "write"));
    metrics.cost += part.get("cost").and_then(Value::as_f64).unwrap_or(0.0);
}

/// Counts a call once it has finished; a repeated part id is the same call.
fn add_tool(metrics: &mut EventMetrics, seen: &mut BTreeSet<String>, part: &Value) {
    let status = part
        .get("state")
        .and_then(|state| state.get("status"))
        .and_then(Value::as_str);
    let failed = match status {
        Some("completed") => false,
        Some("error") => true,
        _ => return,
    };
    let id = part
        .get("id")
        .or_else(|| part.get("callID"))
        .and_then(Value::as_str);
    if id.is_some_and(|id| !seen.insert(id.to_string())) {
        return;
    }

    let tool = part
        .get("tool")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    metrics.tool_calls = metrics.tool_calls.saturating_add(1);
    let entry = metrics.tools.entry(tool).or_insert(0);
    *entry = entry.saturating_add(1);
    if failed {
        metrics.tool_errors = metrics.tool_errors.saturating_add(1);
    }
}

fn error_message(error: Option<&Value>) -> String {
    let Some(error) = error else {
        return "error event without details".into();
    };

    error
        .get("message")
        .or_else(|| error.get("data").and_then(|data| data.get("message")))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            error
                .get("type")
                .or_else(|| error.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| error.to_string())
        .chars()
        .take(MAX_TEXT_CHARS)
        .collect()
}
