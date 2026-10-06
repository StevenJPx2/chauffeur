//! Token counts from OpenCode's session API. OpenCode 2's `run --format json`
//! emits no `step_finish`, so a run's tokens and cost come from the session it
//! created: `/api/session/<id>` for totals, `/context` for each step.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;

use crate::events::{EventMetrics, PROMPT_STEPS, Tokens};
use crate::process::{self, Exit};

const API_TIMEOUT: Duration = Duration::from_secs(60);

/// Fill `metrics` from the session's usage when the events carried none.
/// The replies are saved in `dir` as `session.json` and `context.json`.
///
/// # Errors
/// `opencode api` fails, or a reply is not the expected JSON.
pub fn fill_usage(metrics: &mut EventMetrics, opencode: &Path, dir: &Path) -> Result<(), String> {
    let Some(session) = metrics.session_id.clone() else {
        return Ok(());
    };
    if metrics.steps > 0 {
        return Ok(());
    }

    let totals = api(
        opencode,
        &format!("/api/session/{session}"),
        &dir.join("session.json"),
    )?;
    let context = api(
        opencode,
        &format!("/api/session/{session}/context"),
        &dir.join("context.json"),
    )?;
    let data = &totals["data"];
    let steps: Vec<&Value> = context["data"]
        .as_array()
        .map(|messages| {
            messages
                .iter()
                .filter(|message| message["type"] == "assistant")
                .collect()
        })
        .unwrap_or_default();

    metrics.tokens = tokens(&data["tokens"]);
    metrics.cost = data["cost"].as_f64().unwrap_or(0.0);
    metrics.steps = u64::try_from(steps.len()).unwrap_or(u64::MAX);
    metrics.step_prompts = steps
        .iter()
        .take(PROMPT_STEPS)
        .map(|step| {
            let used = tokens(&step["tokens"]);
            used.input
                .saturating_add(used.cache_read)
                .saturating_add(used.cache_write)
        })
        .collect();

    Ok(())
}

/// Delete every session the runs in `dir/results.jsonl` created, and
/// their child sessions; the number removed. OpenCode keeps the runs'
/// projects: it has no API to remove one.
///
/// # Errors
/// An unreadable results file. A session that cannot be deleted is skipped
/// and named on stderr.
pub fn remove_all(dir: &Path, opencode: &Path) -> Result<usize, String> {
    let results = crate::result::read_jsonl(&dir.join("results.jsonl"))?;
    let mut removed = 0_usize;

    for session in results
        .iter()
        .filter_map(|result| result.events.session_id.as_deref())
    {
        let what = format!("opencode api delete /api/session/{session}");
        let exit = process::run(
            Command::new(opencode)
                .args(["api", "delete", &format!("/api/session/{session}")])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
            &what,
            API_TIMEOUT,
        )?;

        if exit == Exit::Code(Some(0)) {
            removed = removed.saturating_add(1);
        } else {
            eprintln!("chauffeur-bench: {what}: {exit:?}");
        }
    }

    Ok(removed)
}

fn tokens(value: &Value) -> Tokens {
    let count = |value: &Value| value.as_u64().unwrap_or(0);

    Tokens {
        input: count(&value["input"]),
        output: count(&value["output"]),
        reasoning: count(&value["reasoning"]),
        cache_read: count(&value["cache"]["read"]),
        cache_write: count(&value["cache"]["write"]),
    }
}

/// `opencode api get <path>`, written to `file` (a piped reply can be cut
/// short) and parsed.
fn api(opencode: &Path, path: &str, file: &Path) -> Result<Value, String> {
    let out = std::fs::File::create(file)
        .map_err(|error| format!("create {}: {error}", file.display()))?;
    let what = format!("opencode api get {path}");
    let exit = process::run(
        Command::new(opencode)
            .args(["api", "get", path])
            .stdin(Stdio::null())
            .stdout(Stdio::from(out))
            .stderr(Stdio::null()),
        &what,
        API_TIMEOUT,
    )?;

    if exit != Exit::Code(Some(0)) {
        return Err(format!("{what}: {exit:?}"));
    }

    let text = std::fs::read_to_string(file)
        .map_err(|error| format!("read {}: {error}", file.display()))?;

    serde_json::from_str(&text).map_err(|error| format!("{what}: {error}"))
}
