//! `chauffeur audit`: the runs in the audit log, read straight from its files,
//! so it works with no daemon running.

use std::path::Path;

use chauffeur_daemon::audit as log;
use color_eyre::eyre::{Result, WrapErr};
use serde_json::Value;

/// Prints the last `count` runs from the audit log in `state`, oldest first,
/// then, with `follow`, each run as it is written.
pub fn print(state: &Path, count: usize, follow: bool, brief: bool) -> Result<()> {
    let show = |line: &str| {
        if brief {
            println!("{}", summary(line));
        } else {
            println!("{line}");
        }
    };
    if follow {
        // One read for both, so no run falls between the recent and the new.
        return log::follow(state, count, &show)
            .wrap_err_with(|| format!("cannot follow the audit log in {}", state.display()));
    }
    let lines = log::recent(state, count)
        .wrap_err_with(|| format!("cannot read the audit log in {}", state.display()))?;
    for line in &lines {
        show(line);
    }
    Ok(())
}

/// One short line for a run: when, the rule, the session, each step's
/// result, the time it took and what it returned.
fn summary(line: &str) -> String {
    let Ok(run) = serde_json::from_str::<Value>(line) else {
        return line.to_owned();
    };
    let text = |key: &str| run.get(key).and_then(Value::as_str).unwrap_or_default();
    let mut parts = vec![
        local_time(text("at")),
        text("rule").to_owned(),
        text("session").to_owned(),
    ];
    if run.get("cooldown").is_some() {
        parts.push("cooling down".to_owned());
    }
    if let Some(reason) = run.get("refused").and_then(Value::as_str) {
        parts.push(format!("refused: {reason}"));
    }
    if run.get("deadline").is_some() {
        parts.push("missed its deadline".to_owned());
    }
    let steps = run.get("steps").and_then(Value::as_array);
    parts.extend(steps.into_iter().flatten().map(step));
    if let Some(elapsed) = run.get("elapsed_ms").and_then(Value::as_u64) {
        parts.push(format!("{elapsed} ms"));
    }
    if let Some(returned) = run.get("returned").and_then(Value::as_str) {
        parts.push(format!("→ {returned}"));
    }
    parts.join("  ")
}

/// `nonconventional yes 0.94 jev`, or `nonconventional failed: timeout`.
fn step(step: &Value) -> String {
    let id = step.get("step").and_then(Value::as_str).unwrap_or_default();
    if let Some(failed) = step.get("failed").and_then(Value::as_str) {
        return format!("{id} failed: {failed}");
    }
    let answer = step.get("answer").unwrap_or(&Value::Null);
    let outcome = answer
        .get("outcome")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let model = answer
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let p = answer
        .get("p")
        .and_then(|p| p.get(outcome))
        .and_then(Value::as_f64);
    let placeholder = if answer.get("placeholder") == Some(&Value::Bool(true)) {
        " (placeholder)"
    } else {
        ""
    };
    let p = p.map_or_default(|p| format!(" {p:.2}"));
    format!("{id} {outcome}{p} {model}{placeholder}")
}

/// `at` in the machine's time zone, as `13:54:43`.
fn local_time(at: &str) -> String {
    at.parse::<jiff::Timestamp>().map_or_else(
        |_| at.to_owned(),
        |at| {
            at.to_zoned(jiff::tz::TimeZone::system())
                .strftime("%H:%M:%S")
                .to_string()
        },
    )
}
