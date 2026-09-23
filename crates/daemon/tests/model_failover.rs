//! End to end through the engine thread: a usage-limit signal reaches the
//! model router, Jev (over real HTTP) picks a same-tier model, and a switch
//! effect comes back.

mod common;

use chauffeur_core::{AvailableModel, Effect, ModelRef, Signal, SignalKind};
use chauffeur_daemon::{EngineHandle, EngineOptions};
use serde_json::{Value, json};

fn answer(request: &Value) -> Value {
    let criteria = request["questions"]["model-router/choice"]["criteria"]
        .as_object()
        .expect("criteria");

    assert!(criteria.contains_key("openai/gpt-6-sol"));
    assert!(criteria.contains_key("stay"));
    assert!(
        request["state"]
            .as_str()
            .expect("state")
            .contains("Ran tool edit.")
    );

    json!({"model-router/choice": {"type": "choice", "confidence": 0.8, "choice": "openai/gpt-6-luna"}})
}

fn model(provider: &str, id: &str) -> ModelRef {
    ModelRef {
        provider: provider.into(),
        model: id.into(),
    }
}

fn signal(at: u64, kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses_test".into(),
        at,
        kind,
    }
}

#[tokio::test]
async fn usage_limit_switches_to_the_judged_same_tier_model() {
    let (jev, server) = common::serve_jev(answer);
    let config_dir = std::env::temp_dir().join("chauffeur-daemon-test-config-absent");
    let engine = EngineHandle::spawn(EngineOptions {
        // No skills folder: no permission or misuse contracts.
        skills_dir: std::env::temp_dir().join("chauffeur-no-skills-repo"),
        config_dir,
        jev,
        idle_reminders: false,
        state_file: None,
    })
    .await
    .expect("engine starts");

    let tool = signal(
        1,
        SignalKind::ToolResult {
            tool: "edit".into(),
            ok: true,
            input: String::new(),
            error: String::new(),
        },
    );

    assert!(engine.ingest(tool).await.expect("tool signal").is_empty());

    let available = [
        model("anthropic", "claude-opus-5-5"),
        model("anthropic", "claude-haiku-4-5-20251001"),
        model("openai", "gpt-6-sol"),
        model("openai", "gpt-6-luna"),
        model("openai", "gpt-5.5"),
    ]
    .into_iter()
    .map(|model| AvailableModel {
        model,
        usable: true,
    })
    .collect();
    let limit = signal(
        2,
        SignalKind::ModelError {
            model: model("anthropic", "claude-opus-5-5"),
            error_type: "rate_limit_error".into(),
            status: Some(429),
            message: "usage limit".into(),
            tool_executed: false,
            available,
        },
    );
    let effects = engine.ingest(limit).await.expect("limit signal");

    assert_eq!(
        effects,
        vec![Effect::SwitchModel {
            agent_id: "ses_test".into(),
            model: model("openai", "gpt-6-luna")
        }]
    );

    server.join().expect("fixture");
}
