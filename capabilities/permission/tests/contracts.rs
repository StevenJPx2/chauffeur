//! The shipped edit, shell, and outside-directory contracts through the engine.
//! Each judges a request the host would ask about and approves only a
//! confident yes.

use std::sync::{Arc, Mutex};

use chauffeur_capability_permission::{Permission, Skill};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Effect, Engine, PermissionDecision, Question, Resource,
    Signal, SignalKind, SystemOne, SystemOneError,
};

const EDIT_JSON: &[u8] = include_bytes!("../../../skills/permission/workspace-edit-gate.json");
const SHELL_JSON: &[u8] = include_bytes!("../../../skills/permission/workspace-shell-gate.json");
const DIRECTORY_JSON: &[u8] =
    include_bytes!("../../../skills/permission/external-directory-gate.json");

/// System One answers each contract question with this P(yes), or fails.
/// Core's own harm and secret probes are answered no.
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
                value: AnswerValue::Noul(if question.id.starts_with("core/") {
                    0.0
                } else {
                    p
                }),
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
            host_decision: PermissionDecision::Ask,
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

/// The decision one contract reaches when System One answers `p`.
fn judged(
    p: f32,
    contract: &[u8],
    action: &str,
    resource: &str,
) -> (PermissionDecision, Option<String>) {
    let (mut engine, _) = engine(Some(p), contract);

    decision(
        &engine
            .ingest(&request(1, action, resource, resource))
            .unwrap(),
    )
}

#[test]
fn an_unnamed_edit_the_task_needs_is_approved() {
    let (mut engine, calls) = engine(Some(0.9), EDIT_JSON);

    assert_eq!(
        decision(&engine.ingest(&edit(1)).unwrap()).0,
        PermissionDecision::Allow
    );
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn only_a_confident_yes_approves_an_edit() {
    let (unsure, message) = judged(0.6, EDIT_JSON, "edit", "/work/app/app/assets/main.css");

    assert_eq!(unsure, PermissionDecision::Ask);
    assert!(message.unwrap().contains("unsure"));

    let (no, message) = judged(0.2, EDIT_JSON, "edit", "/work/app/app/assets/main.css");

    assert_eq!(no, PermissionDecision::Ask);
    assert!(message.unwrap().contains("did not approve"));
}

#[test]
fn an_edit_outside_the_workspace_is_judged_like_any_other() {
    let (mut engine, calls) = engine(Some(0.9), EDIT_JSON);
    let outside = request(
        1,
        "edit",
        "../sibling/src/lib.rs",
        "/work/sibling/src/lib.rs",
    );

    assert_eq!(
        decision(&engine.ingest(&outside).unwrap()).0,
        PermissionDecision::Allow
    );
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn system_one_failure_asks() {
    for (contract, action) in [
        (EDIT_JSON, "edit"),
        (SHELL_JSON, "shell"),
        (DIRECTORY_JSON, "external_directory"),
    ] {
        let (mut engine, _) = engine(None, contract);
        let (decided, message) = decision(
            &engine
                .ingest(&request(1, action, "x", "/work/app/x"))
                .unwrap(),
        );

        assert_eq!(decided, PermissionDecision::Ask);
        assert!(message.unwrap().contains("could not judge"));
    }
}

#[test]
fn shell_commands_are_approved_only_on_a_confident_yes() {
    assert_eq!(
        judged(0.9, SHELL_JSON, "shell", "cargo check").0,
        PermissionDecision::Allow
    );
    assert_eq!(
        judged(0.5, SHELL_JSON, "shell", "cargo build").0,
        PermissionDecision::Ask
    );
    assert_eq!(
        judged(0.06, SHELL_JSON, "shell", "echo x > main.rs").0,
        PermissionDecision::Ask
    );
}

#[test]
fn an_outside_directory_is_approved_only_when_the_task_needs_it() {
    assert_eq!(
        judged(0.9, DIRECTORY_JSON, "external_directory", "/work/sibling/*").0,
        PermissionDecision::Allow
    );
    assert_eq!(
        judged(
            0.1,
            DIRECTORY_JSON,
            "external_directory",
            "/home/user/.ssh/*"
        )
        .0,
        PermissionDecision::Ask
    );
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
fn a_force_push_to_main_asks_the_user_even_when_jev_would_approve() {
    let (mut engine, calls) = engine(Some(0.95), SHELL_JSON);
    let push = "git push --force-with-lease origin main";
    let (decided, message) = decision(&engine.ingest(&request(1, "shell", push, push)).unwrap());

    assert_eq!(decided, PermissionDecision::Ask);
    assert!(message.unwrap().contains("confirm"));
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn a_request_the_host_allows_is_left_to_the_host() {
    let (mut engine, calls) = engine(Some(0.1), EDIT_JSON);
    let mut allowed = edit(1);

    if let SignalKind::PermissionRequest { host_decision, .. } = &mut allowed.kind {
        *host_decision = PermissionDecision::Allow;
    }

    assert!(engine.ingest(&allowed).unwrap().is_empty());
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
