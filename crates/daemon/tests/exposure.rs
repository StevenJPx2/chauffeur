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
            "skill-exposure/pick",
            "tool-exposure/browser",
            "tool-exposure/github"
        ]
    );

    let skills = questions["skill-exposure/pick"]["criteria"]
        .as_object()
        .expect("skill options");

    assert!(skills.contains_key("hpdp-overlay") && skills.contains_key("none"));
    assert!(
        request["state"]
            .as_str()
            .expect("state")
            .contains("User: fix the lululemon overlay")
    );

    json!({
        "skill-exposure/pick": {"type": "choice", "confidence": 0.9, "choice": "hpdp-overlay"},
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
        // No skills folder: no permission or misuse contracts.
        skills_dir: std::env::temp_dir().join("chauffeur-no-skills-repo"),
        config_dir,
        jev,
        idle_reminders: false,
        state_file: None,
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
        },
    };
    let mut effects = engine.ingest(signal).await.expect("user message");

    effects.sort_by_key(|effect| format!("{effect:?}"));
    assert_eq!(
        effects,
        vec![
            Effect::AttachSkills {
                agent_id: "ses_exposure".into(),
                skills: vec!["hpdp-overlay".into()]
            },
            Effect::HideTools {
                agent_id: "ses_exposure".into(),
                tools: vec!["skill".into(), "github_create_pr".into()]
            },
        ]
    );

    server.join().expect("fixture");
}
