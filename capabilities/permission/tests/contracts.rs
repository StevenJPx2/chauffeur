//! The shipped workspace edit and shell contracts through the engine.

use std::sync::{Arc, Mutex};

use chauffeur_capability_permission::{Permission, Skill};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Effect, Engine, PermissionDecision, Question, Resource,
    Signal, SignalKind, SystemOne, SystemOneError,
};

const EDIT_JSON: &[u8] = include_bytes!("../../../skills/permission/workspace-edit-gate.json");
const SHELL_JSON: &[u8] = include_bytes!("../../../skills/permission/workspace-shell-gate.json");

/// System One answers every question with this P(yes), or fails.
struct Scripted {
    reply: Option<f32>,
    calls: Arc<Mutex<usize>>,
}

impl SystemOne for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn ask(&mut self, _: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        *self.calls.lock().unwrap() += 1;

        let p = self
            .reply
            .ok_or_else(|| SystemOneError("socket closed".into()))?;

        Ok(questions
            .iter()
            .map(|question| Answer {
                id: question.id.clone(),
                value: AnswerValue::Noul(p),
                confidence: None,
            })
            .collect())
    }
}

fn engine(reply: Option<f32>, contract: &[u8]) -> (Engine, Arc<Mutex<usize>>) {
    let calls = Arc::new(Mutex::new(0));
    let skill = Skill::from_json(contract).expect("contract validates");
    let capabilities: Vec<Box<dyn Capability>> = vec![Box::new(Permission::new(vec![skill]))];
    let system_one = Scripted {
        reply,
        calls: Arc::clone(&calls),
    };

    (
        Engine::new(Box::new(system_one), capabilities).expect("engine"),
        calls,
    )
}

fn request(at: u64, action: &str, requested: &str, resolved: &str) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at,
        kind: SignalKind::PermissionRequest {
            action: action.into(),
            resources: vec![Resource {
                requested: requested.into(),
                resolved: resolved.into(),
            }],
            request: String::new(),
            workspace: "/work/app".into(),
            user_requests: vec!["fix the hero spacing on the landing page".into()],
        },
    }
}

/// An edit to a file the user never named, inside the workspace.
fn edit(at: u64) -> Signal {
    request(
        at,
        "edit",
        "app/assets/main.css",
        "/work/app/app/assets/main.css",
    )
}

fn decision(effects: &[Effect]) -> (PermissionDecision, Option<String>) {
    match effects {
        [
            Effect::Permission {
                decision, message, ..
            },
        ] => (*decision, message.clone()),
        other => panic!("expected one permission effect, got {other:?}"),
    }
}

#[test]
fn an_unnamed_edit_in_the_workspace_is_judged_and_allowed() {
    let (mut engine, calls) = engine(Some(0.9), EDIT_JSON);

    assert_eq!(
        decision(&engine.ingest(&edit(1)).unwrap()),
        (PermissionDecision::Allow, None)
    );
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn only_a_confident_no_asks_about_an_edit() {
    let (mut unsure, _) = engine(Some(0.35), EDIT_JSON);
    let (mut no, _) = engine(Some(0.2), EDIT_JSON);

    assert_eq!(
        decision(&unsure.ingest(&edit(1)).unwrap()).0,
        PermissionDecision::Allow
    );

    let (decided, message) = decision(&no.ingest(&edit(1)).unwrap());
    assert_eq!(decided, PermissionDecision::Ask);
    assert!(message.unwrap().contains("unrelated to your stated task"));
}

#[test]
fn an_edit_outside_the_workspace_asks_without_consulting_system_one() {
    let (mut engine, calls) = engine(Some(0.9), EDIT_JSON);
    let outside = request(1, "edit", "/etc/hosts", "/etc/hosts");
    let (decided, message) = decision(&engine.ingest(&outside).unwrap());

    assert_eq!(decided, PermissionDecision::Ask);
    assert_eq!(
        message.as_deref(),
        Some("Keep edits inside the current workspace.")
    );
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn system_one_failure_asks() {
    for contract in [EDIT_JSON, SHELL_JSON] {
        let (mut engine, _) = engine(None, contract);
        let signal = request(
            1,
            if contract == EDIT_JSON {
                "edit"
            } else {
                "shell"
            },
            "x",
            "/work/app/x",
        );
        let (decided, message) = decision(&engine.ingest(&signal).unwrap());

        assert_eq!(decided, PermissionDecision::Ask);
        assert!(message.unwrap().contains("could not judge"));
    }
}

#[test]
fn shell_commands_ask_only_on_a_confident_no() {
    let shell = |p: f32, command: &str| {
        let (mut engine, _) = engine(Some(p), SHELL_JSON);

        decision(
            &engine
                .ingest(&request(1, "shell", command, command))
                .unwrap(),
        )
        .0
    };

    assert_eq!(shell(0.6, "cargo check"), PermissionDecision::Allow);
    // Unsure, as with a multi-part command, still runs.
    assert_eq!(shell(0.18, "cargo build"), PermissionDecision::Allow);
    assert_eq!(shell(0.06, "echo x > main.rs"), PermissionDecision::Ask);
}

#[test]
fn cooldown_asks_without_a_second_judgment() {
    let mut contract: serde_json::Value = serde_json::from_slice(EDIT_JSON).unwrap();
    contract["cooldown_seconds"] = serde_json::json!(30);
    let (mut engine, calls) = engine(Some(0.9), &serde_json::to_vec(&contract).unwrap());

    assert_eq!(
        decision(&engine.ingest(&edit(1)).unwrap()).0,
        PermissionDecision::Allow
    );
    assert_eq!(
        decision(&engine.ingest(&edit(2)).unwrap()).0,
        PermissionDecision::Ask
    );
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn unmatched_actions_leave_the_host_decision() {
    let (mut engine, calls) = engine(Some(0.9), EDIT_JSON);

    assert!(
        engine
            .ingest(&request(1, "read", "a", "/work/app/a"))
            .unwrap()
            .is_empty()
    );
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn contracts_reject_unknown_fields_and_unresolved_allows() {
    let mut unknown: serde_json::Value = serde_json::from_slice(EDIT_JSON).unwrap();
    unknown["unexpected"] = serde_json::json!(true);

    assert!(Skill::from_json(&serde_json::to_vec(&unknown).unwrap()).is_err());

    for outcome in ["uncertain", "missing_evidence", "judge_failure", "cooldown"] {
        let mut value: serde_json::Value = serde_json::from_slice(EDIT_JSON).unwrap();
        value["outcomes"][outcome]["effect"] = serde_json::json!("allow");

        assert!(Skill::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn the_backstop_denies_irreversible_harm_without_consulting_system_one() {
    let (mut engine, calls) = engine(Some(0.9), SHELL_JSON);
    let (decided, message) = decision(
        &engine
            .ingest(&request(1, "shell", "rm -rf /", "rm -rf /"))
            .unwrap(),
    );

    assert_eq!(decided, PermissionDecision::Deny);
    assert!(message.unwrap().contains("irreversible"));
    assert_eq!(*calls.lock().unwrap(), 0);
}
