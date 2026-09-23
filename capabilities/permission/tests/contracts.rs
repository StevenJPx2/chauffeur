//! The shipped workspace edit and shell contracts through the engine.

use std::sync::{Arc, Mutex};

use chauffeur_capability_permission::{Permission, Skill};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Effect, Engine, PermissionDecision, Question, Resource,
    Signal, SignalKind, SystemOne, SystemOneError,
};

const SKILL_JSON: &[u8] = include_bytes!("../../../skills/permission/workspace-edit-gate.json");
const SHELL_JSON: &[u8] = include_bytes!("../../../skills/permission/workspace-shell-gate.json");

#[derive(Clone)]
enum Reply {
    Choice(&'static str, f32),
    Fail,
}

struct Scripted {
    reply: Reply,
    calls: Arc<Mutex<usize>>,
}

impl SystemOne for Scripted {
    fn name(&self) -> &str {
        "scripted"
    }

    fn ask(&mut self, _: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        *self.calls.lock().unwrap() += 1;

        match self.reply {
            Reply::Fail => Err(SystemOneError("socket closed".into())),
            Reply::Choice(choice, confidence) => Ok(questions
                .iter()
                .map(|question| Answer {
                    id: question.id.clone(),
                    value: AnswerValue::Choice(choice.into()),
                    confidence: Some(confidence),
                })
                .collect()),
        }
    }
}

fn engine(reply: Reply) -> (Engine, Arc<Mutex<usize>>) {
    engine_with(reply, SKILL_JSON)
}

fn engine_with(reply: Reply, contract: &[u8]) -> (Engine, Arc<Mutex<usize>>) {
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

fn edit(at: u64, named_by_user: bool) -> Signal {
    let user = if named_by_user {
        "please update README.md"
    } else {
        "tidy things up"
    };

    Signal {
        agent_id: "ses".into(),
        at,
        kind: SignalKind::PermissionRequest {
            action: "edit".into(),
            resources: vec![Resource {
                requested: "README.md".into(),
                resolved: "/work/app/README.md".into(),
            }],
            request: "update the install section".into(),
            workspace: "/work/app".into(),
            user_requests: vec![user.into()],
        },
    }
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
fn confident_in_scope_judgment_allows() {
    let (mut engine, _) = engine(Reply::Choice("within_scope", 0.95));

    assert_eq!(
        decision(&engine.ingest(&edit(1, true)).unwrap()),
        (PermissionDecision::Allow, None)
    );
}

#[test]
fn low_confidence_asks() {
    let (mut engine, _) = engine(Reply::Choice("within_scope", 0.4));
    let (decided, message) = decision(&engine.ingest(&edit(1, true)).unwrap());

    assert_eq!(decided, PermissionDecision::Ask);
    assert!(message.unwrap().contains("uncertain"));
}

#[test]
fn system_one_failure_asks() {
    let (mut engine, _) = engine(Reply::Fail);
    let (decided, message) = decision(&engine.ingest(&edit(1, true)).unwrap());

    assert_eq!(decided, PermissionDecision::Ask);
    assert!(message.unwrap().contains("could not judge"));
}

#[test]
fn missing_evidence_asks_with_the_reminder_without_consulting_system_one() {
    let (mut engine, calls) = engine(Reply::Choice("within_scope", 0.95));
    let (decided, message) = decision(&engine.ingest(&edit(1, false)).unwrap());

    assert_eq!(decided, PermissionDecision::Ask);
    assert_eq!(
        message.as_deref(),
        Some("Name the target file in the request and keep the edit inside the current workspace.")
    );
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn out_of_scope_asks_rather_than_denies() {
    let (mut engine, _) = engine(Reply::Choice("outside_scope", 0.95));
    let (decided, message) = decision(&engine.ingest(&edit(1, true)).unwrap());

    assert_eq!(decided, PermissionDecision::Ask);
    assert!(message.unwrap().contains("unrelated to your stated task"));
}

#[test]
fn every_edit_is_judged_because_the_shipped_contract_has_no_cooldown() {
    let (mut engine, calls) = engine(Reply::Choice("within_scope", 0.95));

    for at in [1, 2] {
        assert_eq!(
            decision(&engine.ingest(&edit(at, true)).unwrap()).0,
            PermissionDecision::Allow
        );
    }
    assert_eq!(*calls.lock().unwrap(), 2);
}

#[test]
fn cooldown_asks_without_a_second_judgment() {
    let mut contract: serde_json::Value = serde_json::from_slice(SKILL_JSON).unwrap();
    contract["cooldown_seconds"] = serde_json::json!(30);
    let (mut engine, calls) = engine_with(
        Reply::Choice("within_scope", 0.95),
        &serde_json::to_vec(&contract).unwrap(),
    );

    assert_eq!(
        decision(&engine.ingest(&edit(1, true)).unwrap()).0,
        PermissionDecision::Allow
    );
    assert_eq!(
        decision(&engine.ingest(&edit(2, true)).unwrap()).0,
        PermissionDecision::Ask
    );
    assert_eq!(*calls.lock().unwrap(), 1);
}

#[test]
fn unmatched_actions_leave_the_host_decision() {
    let (mut engine, calls) = engine(Reply::Choice("within_scope", 0.95));
    let mut signal = edit(1, true);

    if let SignalKind::PermissionRequest { action, .. } = &mut signal.kind {
        *action = "bash".into();
    }

    assert!(engine.ingest(&signal).unwrap().is_empty());
    assert_eq!(*calls.lock().unwrap(), 0);
}

#[test]
fn contracts_reject_unknown_fields_and_unresolved_allows() {
    let mut unknown: serde_json::Value = serde_json::from_slice(SKILL_JSON).unwrap();
    unknown["unexpected"] = serde_json::json!(true);

    assert!(Skill::from_json(&serde_json::to_vec(&unknown).unwrap()).is_err());

    for outcome in ["uncertain", "missing_evidence", "judge_failure", "cooldown"] {
        let mut value: serde_json::Value = serde_json::from_slice(SKILL_JSON).unwrap();
        value["outcomes"][outcome]["effect"] = serde_json::json!("allow");

        assert!(Skill::from_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }
}

#[test]
fn the_backstop_denies_irreversible_harm_without_consulting_system_one() {
    let (mut engine, calls) = engine(Reply::Choice("within_scope", 0.95));
    let mut signal = edit(1, true);

    if let SignalKind::PermissionRequest { request, .. } = &mut signal.kind {
        *request = "clean up with rm -rf / --no-preserve-root".into();
    }

    let (decided, message) = decision(&engine.ingest(&signal).unwrap());

    assert_eq!(decided, PermissionDecision::Deny);
    assert!(message.unwrap().contains("irreversible"));
    assert_eq!(*calls.lock().unwrap(), 0);
}

fn shell(command: &str) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::PermissionRequest {
            action: "shell".into(),
            resources: vec![Resource {
                requested: command.into(),
                resolved: command.into(),
            }],
            request: String::new(),
            workspace: "/work/app".into(),
            user_requests: vec!["fix the failing test".into()],
        },
    }
}

#[test]
fn shell_commands_in_scope_are_allowed_and_others_ask() {
    let (mut allow, _) = engine_with(Reply::Choice("within_scope", 0.65), SHELL_JSON);
    let (mut outside, _) = engine_with(Reply::Choice("outside_scope", 1.0), SHELL_JSON);
    let (mut unsure, _) = engine_with(Reply::Choice("within_scope", 0.34), SHELL_JSON);

    assert_eq!(
        decision(&allow.ingest(&shell("cargo check")).unwrap()).0,
        PermissionDecision::Allow
    );
    assert_eq!(
        decision(&outside.ingest(&shell("echo x > main.rs")).unwrap()).0,
        PermissionDecision::Ask
    );
    assert_eq!(
        decision(&unsure.ingest(&shell("cargo build")).unwrap()).0,
        PermissionDecision::Ask
    );
}
