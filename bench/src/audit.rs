//! Chauffeur's activity in one run, from the daemon's `audit.jsonl`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::stats::mean;

/// What Chauffeur did during a run.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChauffeurMetrics {
    /// Parsed audit records.
    pub records: u64,
    /// Lines that were not JSON objects.
    pub bad_lines: u64,
    /// Records by signal type (`user_message`, `tool_result`, …).
    pub signals: BTreeMap<String, u64>,
    /// Records whose trace shows System One (Jev) was asked.
    pub jev_calls: u64,
    pub jev_elapsed_ms_total: u64,
    pub jev_elapsed_ms_mean: f64,
    /// Traces that carry a System One error.
    pub jev_errors: u64,
    /// Records whose engine run failed.
    pub record_errors: u64,
    /// Skills attached through context effects; keys are the unique skills,
    /// values how often each was attached.
    pub skills: BTreeMap<String, u64>,
    pub tools_hidden: u64,
    pub tools_revealed: u64,
    /// Context effects delivered into the running turn.
    pub steers: u64,
    /// Permission effects by decision (`allow`, `deny`, `ask`).
    pub permissions: BTreeMap<String, u64>,
}

/// Summarize an audit log. Unparseable lines are counted, not fatal.
#[must_use]
pub fn parse(text: &str) -> ChauffeurMetrics {
    let mut metrics = ChauffeurMetrics::default();

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(Value::Object(record)) = serde_json::from_str::<Value>(line) else {
            metrics.bad_lines = metrics.bad_lines.saturating_add(1);
            continue;
        };

        metrics.records = metrics.records.saturating_add(1);
        let signal = record
            .get("signal")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        bump(&mut metrics.signals, signal);
        if record.get("error").is_some_and(|error| !error.is_null()) {
            metrics.record_errors = metrics.record_errors.saturating_add(1);
        }
        if let Some(trace) = record.get("trace") {
            add_trace(&mut metrics, trace);
        }
        for effect in record
            .get("effects")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            add_effect(&mut metrics, effect);
        }
    }

    metrics.jev_elapsed_ms_mean = mean(metrics.jev_elapsed_ms_total, metrics.jev_calls);

    metrics
}

fn add_trace(metrics: &mut ChauffeurMetrics, trace: &Value) {
    let non_empty = |key: &str| {
        trace
            .get(key)
            .and_then(Value::as_array)
            .is_some_and(|items| !items.is_empty())
    };
    let elapsed = trace.get("elapsed_ms").and_then(Value::as_u64).unwrap_or(0);
    let errored = trace.get("error").is_some_and(|error| !error.is_null());

    if non_empty("answers") || non_empty("questions") || elapsed > 0 || errored {
        metrics.jev_calls = metrics.jev_calls.saturating_add(1);
        metrics.jev_elapsed_ms_total = metrics.jev_elapsed_ms_total.saturating_add(elapsed);
    }
    if errored {
        metrics.jev_errors = metrics.jev_errors.saturating_add(1);
    }
}

fn add_effect(metrics: &mut ChauffeurMetrics, effect: &Value) {
    let names = |key: &str| {
        effect
            .get(key)
            .and_then(Value::as_array)
            .map_or(0, |items| u64::try_from(items.len()).unwrap_or(u64::MAX))
    };

    match effect.get("type").and_then(Value::as_str) {
        Some("context") => {
            for skill in effect
                .get("skills")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                bump(&mut metrics.skills, skill);
            }
            if effect.get("delivery").and_then(Value::as_str) == Some("steer") {
                metrics.steers = metrics.steers.saturating_add(1);
            }
        }
        Some("tools") => {
            metrics.tools_hidden = metrics.tools_hidden.saturating_add(names("hide"));
            metrics.tools_revealed = metrics.tools_revealed.saturating_add(names("reveal"));
        }
        Some("permission") => {
            let decision = effect
                .get("decision")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            bump(&mut metrics.permissions, decision);
        }
        _ => {}
    }
}

fn bump(map: &mut BTreeMap<String, u64>, key: &str) {
    let entry = map.entry(key.to_string()).or_insert(0);
    *entry = entry.saturating_add(1);
}
