//! `chauffeur audit [N]`: the last N decisions from the daemon's audit log,
//! one line each.

use std::path::Path;

use serde_json::Value;

const DEFAULT_LINES: usize = 20;
const MAX_LINES: usize = 1_000;
const MAX_AUDIT_BYTES: u64 = 9 * 1024 * 1024;

pub fn run(args: &[String], path: &Path) -> Result<(), String> {
    let count = args.first().map_or(Ok(DEFAULT_LINES), |value| {
        value
            .parse::<usize>()
            .map_err(|_| format!("usage: chauffeur audit [N] (N up to {MAX_LINES})"))
    })?;
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("no audit log at {}: {error}", path.display()))?;

    if metadata.len() > MAX_AUDIT_BYTES {
        return Err(format!(
            "{} exceeds {MAX_AUDIT_BYTES} bytes",
            path.display()
        ));
    }

    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let lines: Vec<&str> = text.lines().collect();

    for line in &lines[lines.len().saturating_sub(count.min(MAX_LINES))..] {
        println!(
            "{}",
            render(&serde_json::from_str(line).unwrap_or(Value::Null))
        );
    }

    Ok(())
}

/// `12:04:31 ses_f32… tool_result shell {...} → nudge | 6 asked, 412 ms`
fn render(record: &Value) -> String {
    let at = record["at"].as_u64().unwrap_or(0);
    let clock = format!("{:02}:{:02}:{:02}", at / 3_600 % 24, at / 60 % 60, at % 60);
    let agent: String = record["agent_id"]
        .as_str()
        .unwrap_or("?")
        .chars()
        .take(12)
        .collect();
    let effects: Vec<String> = record["effects"]
        .as_array()
        .map(|effects| effects.iter().map(effect).collect())
        .unwrap_or_default();
    let trace = &record["trace"];
    let mut notes = vec![format!(
        "{} asked, {} ms",
        trace["questions"].as_array().map_or(0, Vec::len),
        trace["elapsed_ms"].as_u64().unwrap_or(0)
    )];

    for (key, label) in [("veto", "vetoed"), ("error", "Jev failed")] {
        if let Some(reason) = trace[key].as_str() {
            notes.push(format!("{label}: {reason}"));
        }
    }

    format!(
        "{clock} UTC {agent}… {} {} → {} | {}",
        record["signal"].as_str().unwrap_or("?"),
        record["detail"].as_str().unwrap_or(""),
        if effects.is_empty() {
            "nothing".into()
        } else {
            effects.join(", ")
        },
        notes.join("; ")
    )
}

fn effect(effect: &Value) -> String {
    let name = effect["type"].as_str().unwrap_or("?");
    let about = [
        "skills",
        "tools",
        "namespaces",
        "decision",
        "rule_id",
        "model",
    ]
    .iter()
    .find_map(|key| {
        let value = &effect[*key];

        (!value.is_null()).then(|| match value {
            Value::String(text) => text.clone(),
            Value::Array(items) if items.len() > 3 => format!("{} items", items.len()),
            other => other.to_string(),
        })
    });

    about.map_or_else(|| name.to_string(), |about| format!("{name} {about}"))
}
