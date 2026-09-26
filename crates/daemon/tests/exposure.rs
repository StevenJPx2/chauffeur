//! A first user message routes skill and tool exposure through one Jev
//! request over real HTTP and returns both effects.

mod common;

use chauffeur_core::{CatalogEntry, Effect, Signal, SignalKind};
use chauffeur_daemon::{EngineHandle, EngineOptions};
use serde_json::{Value, json};

fn answer(request: &Value) -> Value {
    let questions = request["questions"].as_object().expect("questions");
    let mut ids: Vec<&String> = questions.keys().collect();

    ids.sort();
    assert_eq!(
        ids,
        vec![
            "skill-exposure/skill:hpdp-overlay",
            "skill-exposure/skill:slidev",
            "tool-exposure/browser",
            "tool-exposure/github"
        ]
    );
    assert_eq!(questions["skill-exposure/skill:slidev"]["type"], "noul");
    assert!(
        request["state"]
            .as_str()
            .expect("state")
            .contains("User: fix the lululemon overlay")
    );

    json!({
        "skill-exposure/skill:hpdp-overlay": {"type": "noul", "noul": 0.9},
        "skill-exposure/skill:slidev": {"type": "noul", "noul": 0.05},
        "tool-exposure/browser": {"type": "noul", "confidence": 0.8, "noul": 0.9},
        "tool-exposure/github": {"type": "noul", "confidence": 0.8, "noul": 0.1},
    })
}

fn entry(id: &str, description: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        description: description.into(),
        bytes: 2_000,
    }
}

#[tokio::test]
async fn first_user_message_attaches_skills_and_hides_unneeded_tools() {
    let (jev, server) = common::serve_jev(answer);
    let config_dir = std::env::temp_dir().join("chauffeur-exposure-test-config-absent");
    let engine = EngineHandle::spawn(EngineOptions {
        // No skills folder: no permission contracts or rules.
        skills_dir: std::env::temp_dir().join("chauffeur-no-skills-repo"),
        config_dir,
        jev,
        idle_reminders: false,
        sourcefed: None,
        state_file: None,
        audit_file: None,
    })
    .await
    .expect("engine starts");
    let signal = Signal {
        agent_id: "ses_exposure".into(),
        at: 1,
        kind: SignalKind::UserMessage {
            text: "fix the lululemon overlay size notice".into(),
            first_in_context: true,
            skills: vec![
                entry("hpdp-overlay", "Develop and ship HPDP Overlay changes"),
                entry("slidev", "Build slide decks"),
            ],
            tools: vec![
                entry("read", "Read a file"),
                entry("skill", "Load a skill"),
                entry("browser_open", "Open a browser tab"),
                entry("github_create_pr", "Open a pull request"),
            ],
            model: None,
            code_mode: Vec::new(),
            workspace: String::new(),
        },
    };
    let mut effects = engine.ingest(signal).await.expect("user message");

    effects.sort_by_key(|effect| format!("{effect:?}"));
    assert_eq!(
        effects,
        vec![
            Effect::Context {
                agent_id: "ses_exposure".into(),
                delivery: chauffeur_core::Delivery::Prompt,
                label: "skill hpdp-overlay".into(),
                skills: vec!["hpdp-overlay".into()],
                text: None
            },
            Effect::Tools {
                agent_id: "ses_exposure".into(),
                hide: vec!["skill".into(), "github_create_pr".into()],
                reveal: Vec::new()
            },
        ]
    );

    server.join().expect("fixture");
}
