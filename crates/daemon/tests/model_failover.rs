//! End to end through the engine thread: a usage-limit signal reaches the
//! model router, Jev (over real HTTP) picks a same-tier model, and a switch
//! effect comes back.

mod common;

use chauffeur_core::{AvailableModel, Effect, ModelRef, Signal, SignalKind};
use chauffeur_daemon::{EngineHandle, EngineOptions};
use serde_json::{Value, json};

/// Jev's answer: `expected` must be offered, `other_tier` must not.
fn answer(request: &Value, expected: &str, other_tier: &str) -> Value {
    let criteria = request["questions"]["model-router/choice"]["criteria"]
        .as_object()
        .expect("criteria");

    assert!(criteria.contains_key(expected), "{criteria:?}");
    assert!(!criteria.contains_key(other_tier), "{criteria:?}");
    assert!(criteria.contains_key("stay"));
    assert!(
        request["state"]
            .as_str()
            .expect("state")
            .contains("Ran tool edit.")
    );

    json!({"model-router/choice": {"type": "choice", "confidence": 0.8, "choice": expected}})
}

fn model(provider: &str, id: &str) -> ModelRef {
    ModelRef {
        provider: provider.into(),
        model: id.into(),
        variant: None,
    }
}

fn high(provider: &str, id: &str) -> ModelRef {
    ModelRef {
        variant: Some("high".into()),
        ..model(provider, id)
    }
}

fn signal(at: u64, kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses_test".into(),
        at,
        kind,
    }
}

/// A usage limit on `from` switches the agent to `to`, Jev having been
/// offered `to` and not `other_tier`.
async fn fails_over(from: ModelRef, to: ModelRef, other_tier: &str) {
    let expected = format!("{}/{}#high", to.provider, to.model);
    let other_tier = other_tier.to_owned();
    let (jev, server) = common::serve_jev(move |request| answer(request, &expected, &other_tier));
    let config_dir = std::env::temp_dir().join("chauffeur-daemon-test-config-absent");
    let engine = EngineHandle::spawn(EngineOptions {
        // No skills folder: no permission contracts or rules.
        skills_dir: std::env::temp_dir().join("chauffeur-no-skills-repo"),
        config_dir,
        judge: chauffeur_daemon::JudgeConfig::jev(jev),
        system_two: None,
        idle_reminders: false,
        sourcefed: None,
        disabled: Vec::new(),
        state_file: None,
        audit_file: None,
    })
    .await
    .expect("engine starts");

    let tool = signal(
        1,
        SignalKind::ToolResult {
            tool: "edit".into(),
            ok: true,
            workspace: String::new(),
            subagent: false,
            input: String::new(),
            error: String::new(),
            user_request: String::new(),
            evidence: String::new(),
            candidates: Vec::new(),
        },
    );

    assert!(engine.ingest(tool).await.expect("tool signal").is_empty());

    let available = [
        model("anthropic", "claude-fable-5-2"),
        model("anthropic", "claude-opus-5-5"),
        model("anthropic", "claude-sonnet-5-5"),
        model("openai", "gpt-6-astra"),
        model("openai", "gpt-6.1-sol"),
        model("openai", "gpt-6-luna"),
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
            model: from,
            error_type: "rate_limit_error".into(),
            status: Some(429),
            message: "rate limit exceeded, retry in 20s".into(),
            tool_executed: false,
            available,
        },
    );
    let effects = engine.ingest(limit).await.expect("limit signal");

    assert_eq!(
        effects,
        vec![Effect::Model {
            agent_id: "ses_test".into(),
            model: Some(to)
        }]
    );

    server.join().expect("fixture");
}

#[tokio::test]
async fn a_frontier_supervisor_fails_over_to_frontier_on_the_other_provider() {
    fails_over(
        high("anthropic", "claude-fable-5-2"),
        high("openai", "gpt-6-astra"),
        "openai/gpt-6.1-sol#high",
    )
    .await;
}

#[tokio::test]
async fn a_balanced_worker_fails_over_to_balanced_on_the_other_provider() {
    fails_over(
        high("anthropic", "claude-opus-5-5"),
        high("openai", "gpt-6.1-sol"),
        "openai/gpt-6-astra#high",
    )
    .await;
}
