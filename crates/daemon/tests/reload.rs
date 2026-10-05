//! A rule added to the skills folder while the daemon runs applies to the
//! next signal, without a restart.

mod common;

use chauffeur_core::{Signal, SignalKind};
use chauffeur_daemon::{EngineHandle, EngineOptions};
use serde_json::{Map, Value, json};

const RULE: &str = r#"{
  "schema_version": 2,
  "id": "cat-to-read",
  "name": "Reading files through the shell",
  "on": "tool_result",
  "when": { "tools": ["shell"] },
  "steps": [
    { "id": "misuse", "question": "Does this call print a file to read it?", "yes_at_or_above": 0.65, "minimum_confidence": 0.4 }
  ],
  "then": { "delivery": "steer", "text": "Chauffeur: View files with the read tool." }
}"#;

/// Yes to every yes/no question asked.
fn yes(request: &Value) -> Value {
    let answers: Map<String, Value> = request["questions"]
        .as_object()
        .expect("questions")
        .keys()
        .map(|id| (id.clone(), json!({ "type": "noul", "noul": 0.95 })))
        .collect();

    Value::Object(answers)
}

fn cat(at: u64) -> Signal {
    Signal {
        agent_id: "ses_reload".into(),
        at,
        kind: SignalKind::ToolResult {
            tool: "shell".into(),
            ok: true,
            workspace: String::new(),
            subagent: false,
            input: r#"{"command":"cat src/main.rs"}"#.into(),
            error: String::new(),
            user_request: String::new(),
            evidence: String::new(),
            candidates: Vec::new(),
        },
    }
}

#[tokio::test]
async fn a_rule_added_while_running_applies_to_the_next_signal() {
    let root = std::env::temp_dir().join(format!("chauffeur-reload-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let skills_dir = root.join("skills");
    std::fs::create_dir_all(skills_dir.join("rules")).expect("rules folder");
    let (jev, server) = common::serve_jev(yes);
    let engine = EngineHandle::spawn(EngineOptions {
        config_dir: root.join("config"),
        skills_dir: skills_dir.clone(),
        jev,
        idle_reminders: false,
        sourcefed: None,
        disabled: Vec::new(),
        state_file: None,
        audit_file: None,
    })
    .await
    .expect("engine starts");

    let before = engine.ingest(cat(1)).await.expect("first tool result");
    assert!(before.is_empty(), "no rule yet: {before:?}");

    std::fs::write(skills_dir.join("rules/cat-to-read.json"), RULE).expect("write rule");
    // Checks are at most once a second.
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;

    let after = engine.ingest(cat(2)).await.expect("second tool result");
    let _ = std::fs::remove_dir_all(&root);
    // Not joined: the fake Jev waits forever if the new rule never asks.
    drop(server);

    assert!(
        format!("{after:?}").contains("View files with the read tool"),
        "the new rule steers: {after:?}"
    );
}
