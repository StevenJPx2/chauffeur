//! The audit log: one JSONL record per decision, so a user can see why
//! Chauffeur did something. Bounded (one rotation at 8 MiB) and redacted.

use std::io::Write;
use std::path::Path;

use chauffeur_core::{Effect, Signal, SignalKind, Trace, redact_secrets};
use serde_json::json;

const MAX_AUDIT_BYTES: u64 = 8 * 1024 * 1024;
const MAX_DETAIL_CHARS: usize = 200;

/// Append a record when the signal asked System One, was vetoed, produced
/// effects, or failed. Signals that changed nothing are not logged.
pub fn append(path: &Path, signal: &Signal, trace: &Trace, result: &Result<Vec<Effect>, String>) {
    let quiet = trace.questions.is_empty()
        && trace.veto.is_none()
        && matches!(result, Ok(effects) if effects.is_empty());

    if quiet {
        return;
    }

    let record = json!({
        "at": signal.at,
        "agent_id": signal.agent_id,
        "signal": kind_name(&signal.kind),
        "detail": detail(&signal.kind),
        "trace": trace,
        "effects": result.as_ref().ok(),
        "error": result.as_ref().err(),
    });

    if let Err(error) = write(path, &record) {
        eprintln!(
            "chauffeur: audit record not written to {}: {error}",
            path.display()
        );
    }
}

fn write(path: &Path, record: &serde_json::Value) -> std::io::Result<()> {
    if std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > MAX_AUDIT_BYTES) {
        std::fs::rename(path, path.with_extension("1.jsonl"))?;
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    writeln!(file, "{record}")
}

fn kind_name(kind: &SignalKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value["type"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// A short, redacted summary of what the signal was about.
fn detail(kind: &SignalKind) -> String {
    let text = match kind {
        SignalKind::UserMessage { text, .. } => text.clone(),
        SignalKind::ToolResult { tool, input, .. } => format!("{tool} {input}"),
        SignalKind::PermissionRequest {
            action, resources, ..
        } => {
            let targets: Vec<&str> = resources
                .iter()
                .map(|resource| resource.requested.as_str())
                .collect();

            format!("{action} {}", targets.join(", "))
        }
        SignalKind::ModelError {
            model, error_type, ..
        } => format!("{} {error_type}", model.key()),
        SignalKind::IntegrationEvent {
            source,
            kind,
            summary,
            ..
        } => format!("{source} {kind}: {summary}"),
        SignalKind::ModelSucceeded { model } => model.key(),
        SignalKind::TurnEnd => String::new(),
    };

    redact_secrets(&text)
        .chars()
        .take(MAX_DETAIL_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(kind: SignalKind) -> Signal {
        Signal {
            agent_id: "ses".into(),
            at: 7,
            kind,
        }
    }

    #[test]
    fn records_decisions_redacted_and_skips_quiet_signals() {
        let dir = std::env::temp_dir().join(format!("chauffeur-audit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("audit.jsonl");
        let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
        let tool = signal(SignalKind::ToolResult {
            tool: "shell".into(),
            ok: true,
            input: format!("{{\"command\":\"echo {secret}\"}}"),
            error: String::new(),
        });
        let nudge = Effect::Context {
            agent_id: "ses".into(),
            delivery: chauffeur_core::Delivery::Steer,
            label: "shell check".into(),
            skills: Vec::new(),
            text: Some("Chauffeur: use rg".into()),
        };

        append(
            &path,
            &signal(SignalKind::TurnEnd),
            &Trace::default(),
            &Ok(Vec::new()),
        );
        append(&path, &tool, &Trace::default(), &Ok(vec![nudge]));

        let written = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = written.lines().collect();

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"signal\":\"tool_result\""));
        assert!(lines[0].contains("Chauffeur: use rg"));
        assert!(!lines[0].contains(secret));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
