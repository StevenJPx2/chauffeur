use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chauffeur_core::{
    Branch, Effect, EvaluationStatus, InMemoryReminderQueue, Runtime, Skill, SkillContext,
    SkillEvaluateResult, dispatch_method,
};
use chauffeur_judge_laya::LayaJudge;

const SKILL_JSON: &[u8] = include_bytes!("../../../skills/workspace-edit-gate.json");
const POSITIVE_CHOICE: &str = r#"{"model":"coreml","answers":{"edit_is_in_user_scope":{"type":"choice","confidence":0.95,"action":{"act_probability":1},"choice":"within_scope"}},"usage":{"input_tokens":10,"output_tokens":0}}"#;

enum Prediction {
    Respond(&'static str),
    Close,
    RespondIfAsked(&'static str),
}

#[test]
fn low_confidence_choice_uses_ask_outcome() {
    let (path, server) = fake_laya(Prediction::Respond(
        r#"{"model":"coreml","answers":{"edit_is_in_user_scope":{"type":"choice","confidence":0.4,"action":{"act_probability":1},"choice":"within_scope"}},"usage":{"input_tokens":10,"output_tokens":0}}"#,
    ));
    let runtime = runtime(&path);
    let result = evaluate(&runtime, context(true));

    assert_eq!(result.status, EvaluationStatus::Evaluated);
    assert_eq!(result.branch, Some(chauffeur_core::Branch::Uncertain));
    assert_eq!(result.effect, Some(Effect::Ask));

    server.join().expect("fake Laya server exits");
    remove_socket(path);
}

#[test]
fn high_confidence_positive_choice_uses_allow_outcome() {
    let (path, server) = fake_laya(Prediction::Respond(POSITIVE_CHOICE));
    let runtime = runtime(&path);
    let result = evaluate(&runtime, context(true));

    assert_eq!(result.status, EvaluationStatus::Evaluated);
    assert_eq!(result.branch, Some(Branch::Positive));
    assert_eq!(result.effect, Some(Effect::Allow));

    server.join().expect("fake Laya server exits");
    remove_socket(path);
}

#[test]
fn cooldown_uses_declared_outcome_without_a_second_prediction() {
    let (path, server) = fake_laya(Prediction::Respond(POSITIVE_CHOICE));
    let runtime = runtime(&path);
    let first = evaluate(&runtime, context(true));
    let mut next_context = context(true);
    next_context.occurred_at = 101;
    let second = evaluate(&runtime, next_context);

    assert_eq!(first.effect, Some(Effect::Allow));
    assert_eq!(second.status, EvaluationStatus::Cooldown);
    assert_eq!(second.effect, Some(Effect::Ask));

    server.join().expect("fake Laya server sees one prediction");
    remove_socket(path);
}

#[test]
fn laya_socket_failure_uses_judge_failure_ask_outcome() {
    let (path, server) = fake_laya(Prediction::Close);
    let runtime = runtime(&path);
    let result = evaluate(&runtime, context(true));

    assert_eq!(result.status, EvaluationStatus::JudgeFailure);
    assert_eq!(result.effect, Some(Effect::Ask));
    assert!(
        result
            .message
            .as_deref()
            .is_some_and(|message| message.contains("could not evaluate"))
    );

    server.join().expect("fake Laya server exits");
    remove_socket(path);
}

#[test]
fn missing_hard_evidence_returns_a_deterministic_reminder_without_laya() {
    let (path, server) = fake_laya(Prediction::RespondIfAsked(POSITIVE_CHOICE));
    let runtime = runtime(&path);
    let result = evaluate(&runtime, context(false));

    assert_eq!(result.status, EvaluationStatus::MissingEvidence);
    assert_eq!(result.effect, Some(Effect::Ask));
    assert_eq!(
        result.reminder.as_deref(),
        Some("Name the target file in the request and keep the edit inside the current workspace.")
    );

    server
        .join()
        .expect("fake Laya server exits without receiving a prediction");
    remove_socket(path);
}

#[test]
fn skill_schema_rejects_unknown_fields() {
    let mut value: serde_json::Value =
        serde_json::from_slice(SKILL_JSON).expect("sample skill JSON is valid");
    value
        .as_object_mut()
        .expect("top-level skill is an object")
        .insert("unexpected".into(), serde_json::json!(true));
    let encoded = serde_json::to_vec(&value).expect("skill serializes");

    assert!(Skill::from_json(&encoded).is_err());
}

#[test]
fn unresolved_outcomes_cannot_allow_the_permission() {
    for outcome in ["uncertain", "missing_evidence", "judge_failure", "cooldown"] {
        let mut value: serde_json::Value =
            serde_json::from_slice(SKILL_JSON).expect("sample skill JSON is valid");
        value["outcomes"][outcome]["effect"] = serde_json::json!("allow");
        let encoded = serde_json::to_vec(&value).expect("skill serializes");

        assert!(Skill::from_json(&encoded).is_err());
    }
}

fn runtime(path: &Path) -> Runtime {
    let skill = Skill::from_json(SKILL_JSON).expect("sample skill validates");
    let judge = LayaJudge::connect(path).expect("fake Laya reports healthy");

    Runtime::with_skills(
        Vec::new(),
        vec![skill],
        Box::new(judge),
        Arc::new(InMemoryReminderQueue::default()),
    )
    .expect("runtime accepts validated skill")
}

fn context(include_hard_evidence: bool) -> SkillContext {
    let evidence = if include_hard_evidence {
        HashMap::from([
            ("resource_present".into(), true),
            ("resource_within_workspace".into(), true),
            ("resource_named_by_user".into(), true),
        ])
    } else {
        HashMap::new()
    };

    SkillContext {
        event: "permission.evaluate".into(),
        action: "edit".into(),
        agent_id: "test-session".into(),
        occurred_at: 100,
        state: "user requested a README change".into(),
        evidence,
    }
}

fn evaluate(runtime: &Runtime, context: SkillContext) -> chauffeur_core::SkillResult {
    let response = dispatch_method(
        runtime,
        "skill.evaluate",
        serde_json::json!({ "skill_id": "workspace-edit-gate", "context": context }),
    )
    .expect("skill evaluation RPC succeeds");

    serde_json::from_value::<SkillEvaluateResult>(response)
        .expect("skill evaluation response matches its wire type")
        .evaluation
}

fn fake_laya(prediction: Prediction) -> (PathBuf, JoinHandle<()>) {
    static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);
    let sequence = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "chauffeur-laya-{}-{sequence}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind fake Laya socket");
    let server = thread::spawn(move || serve_fake_laya(listener, prediction));

    (path, server)
}

fn serve_fake_laya(listener: UnixListener, prediction: Prediction) {
    let (stream, _) = listener.accept().expect("accept Laya connection");
    let mut reader = BufReader::new(stream);
    let health = read_line(&mut reader);
    let request: serde_json::Value = serde_json::from_str(&health).expect("health request JSON");

    assert_eq!(request["op"], "health");
    write_line(
        reader.get_mut(),
        r#"{"status":"ok","model":"fixture","warm":true}"#,
    );

    if matches!(&prediction, Prediction::RespondIfAsked(_)) {
        reader
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(100)))
            .expect("set fixture timeout");
    }

    let encoded = match read_prediction(&mut reader, &prediction) {
        Some(line) => line,
        None => return,
    };
    let request: serde_json::Value =
        serde_json::from_str(&encoded).expect("prediction request JSON");

    assert_eq!(
        request["questions"]["edit_is_in_user_scope"]["type"],
        "choice"
    );
    assert_eq!(
        request["questions"]["edit_is_in_user_scope"]["criteria"][0],
        "within_scope"
    );

    match prediction {
        Prediction::Respond(response) | Prediction::RespondIfAsked(response) => {
            write_line(reader.get_mut(), response);
        }
        Prediction::Close => {}
    }
}

fn read_prediction(reader: &mut BufReader<UnixStream>, prediction: &Prediction) -> Option<String> {
    let mut line = String::new();

    match reader.read_line(&mut line) {
        Ok(0) => None,
        Ok(_) => Some(line),
        Err(error)
            if matches!(prediction, Prediction::RespondIfAsked(_))
                && matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
        {
            None
        }
        Err(error) => panic!("read Laya prediction: {error}"),
    }
}

fn read_line(reader: &mut BufReader<UnixStream>) -> String {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read Laya request");

    line
}

fn write_line(stream: &mut UnixStream, line: &str) {
    stream
        .write_all(line.as_bytes())
        .expect("write Laya response");
    stream.write_all(b"\n").expect("terminate Laya response");
}

fn remove_socket(path: PathBuf) {
    let _ = std::fs::remove_file(path);
}
