//! Rules through the engine: shipped tool-result steers, turn-end reminders
//! over the session, and a project's own rules scoped to its worktree.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use chauffeur_capability_rules::{MAX_DELIVERIES, Rule, Rules, load_dir, load_project};
use chauffeur_core::{Answer, AnswerValue, Delivery, Effect, Engine, Signal, SignalKind, Step};
use serde_json::{Value, json};

fn rule(id: &str, on: &str, when: Value, steps: &[&str], delivery: &str) -> Value {
    json!({
        "schema_version": 2,
        "id": id,
        "name": id.to_uppercase(),
        "on": on,
        "when": when,
        "steps": steps.iter().map(|step| json!({
            "id": step,
            "question": format!("Does {id} {step} hold?"),
            "yes_at_or_above": 0.7,
            "minimum_confidence": 0.4,
        })).collect::<Vec<_>>(),
        "then": { "delivery": delivery, "text": format!("do {id}") },
    })
}

fn misuse(id: &str) -> Value {
    rule(
        id,
        "tool_result",
        json!({ "tools": ["shell"] }),
        &["misuse"],
        "steer",
    )
}

fn reminder(id: &str, when: Value) -> Value {
    rule(id, "turn_end", when, &["holds"], "resume")
}

fn with(mut value: Value, field: &str, set: Value) -> Value {
    value[field] = set;
    value
}

fn parsed(value: &Value) -> Result<Rule, String> {
    Rule::from_json(&serde_json::to_vec(value).unwrap())
}

fn engine(rules: &[Value]) -> Engine {
    let rules = rules.iter().map(|rule| parsed(rule).unwrap()).collect();

    Engine::hosted(vec![Box::new(Rules::new(rules))]).unwrap()
}

fn signal(at: u64, kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at,
        kind,
    }
}

fn call(at: u64, tool: &str, workspace: &str) -> Signal {
    signal(
        at,
        SignalKind::ToolResult {
            tool: tool.into(),
            ok: true,
            workspace: workspace.into(),
            input: r#"{"command":"python3 patch.py"}"#.into(),
            error: String::new(),
            user_request: String::new(),
            evidence: String::new(),
            candidates: Vec::new(),
        },
    )
}

fn turn_end(at: u64, workspace: &str, request: &str) -> Signal {
    signal(
        at,
        SignalKind::TurnEnd {
            workspace: workspace.into(),
            user_request: request.into(),
        },
    )
}

fn user_message(at: u64) -> Signal {
    signal(
        at,
        SignalKind::UserMessage {
            text: "A new task".into(),
            first_in_context: false,
            skills: vec![],
            tools: vec![],
            model: None,
            code_mode: vec![],
            workspace: String::new(),
        },
    )
}

fn integration(at: u64, source: &str, kind: &str) -> Signal {
    signal(
        at,
        SignalKind::IntegrationEvent {
            source: source.into(),
            kind: kind.into(),
            summary: format!("{source} {kind}"),
            body: String::new(),
            actionable: true,
            monitor: String::new(),
        },
    )
}

/// The question IDs `signal` asks, or none when it settles.
fn asked(engine: &mut Engine, signal: &Signal) -> Vec<String> {
    match engine.begin(signal).unwrap() {
        Step::Ask { questions, .. } => questions.into_iter().map(|question| question.id).collect(),
        Step::Done(effects) => {
            assert!(effects.is_empty(), "settled with {effects:?}");
            Vec::new()
        }
    }
}

fn noul(id: &str, probability: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Noul(probability),
        confidence: None,
    }
}

/// Answer every pending question with `probability`.
fn answer_all(engine: &mut Engine, ids: &[String], probability: f32) -> Step {
    engine.finish(Ok(ids.iter().map(|id| noul(id, probability)).collect()), 1)
}

fn labels(step: &Step) -> Vec<&str> {
    let Step::Done(effects) = step else {
        panic!("expected effects, got {step:?}")
    };

    effects
        .iter()
        .map(|effect| match effect {
            Effect::Context { label, .. } => label.as_str(),
            other => panic!("expected context, got {other:?}"),
        })
        .collect()
}

// Tool-result rules.

#[test]
fn only_watched_tools_are_judged_with_the_call_in_the_question() {
    let mut engine = engine(&[misuse("hand-rolled-patch"), misuse("slack-via-browser")]);

    assert!(asked(&mut engine, &call(1, "edit", "")).is_empty());

    let Step::Ask { questions, .. } = engine.begin(&call(2, "shell", "")).unwrap() else {
        panic!("expected questions")
    };
    let ids: Vec<&str> = questions
        .iter()
        .map(|question| question.id.as_str())
        .collect();

    assert_eq!(
        ids,
        [
            "rules/hand-rolled-patch/misuse",
            "rules/slack-via-browser/misuse"
        ]
    );
    assert!(
        questions[0]
            .instructions
            .contains("Does hand-rolled-patch misuse hold?")
    );
    assert!(questions[0].instructions.contains("python3 patch.py"));
}

#[test]
fn a_confirmed_misuse_steers_hands_over_its_skill_and_cools_down() {
    let mut slack = with(misuse("slack-via-browser"), "cooldown_seconds", json!(120));

    slack["then"]["skill"] = json!("slack-cli");

    let mut engine = engine(&[
        with(misuse("hand-rolled-patch"), "cooldown_seconds", json!(120)),
        slack,
    ]);
    let ids = asked(&mut engine, &call(1, "shell", ""));
    let step = engine.finish(Ok(vec![noul(&ids[0], 0.2), noul(&ids[1], 0.9)]), 1);

    assert_eq!(
        step,
        Step::Done(vec![Effect::Context {
            agent_id: "ses".into(),
            delivery: Delivery::Steer,
            label: "SLACK-VIA-BROWSER".into(),
            skills: vec!["slack-cli".into()],
            text: Some("do slack-via-browser".into()),
        }])
    );
    // Cooling down, only the other rule is still asked; a failed judgment is silent.
    let ids = asked(&mut engine, &call(60, "shell", ""));

    assert_eq!(ids, ["rules/hand-rolled-patch/misuse"]);
    assert_eq!(engine.finish(Err("down".into()), 1), Step::Done(vec![]));
}

// Turn-end rules over the session.

#[test]
fn session_gates_use_the_tools_the_agent_ran() {
    let create_pr = reminder(
        "create-pr",
        json!({ "history": "session", "status": ["implementing"], "tools_not_called": ["github_open_pr"] }),
    );
    let fix_ci = reminder(
        "fix-ci",
        json!({ "history": "session", "status": ["in_review"] }),
    );
    let mut engine = engine(&[create_pr, fix_ci]);

    asked(&mut engine, &call(1, "edit", ""));
    assert_eq!(
        asked(&mut engine, &turn_end(2, "", "")),
        ["rules/create-pr/holds"]
    );

    // A new request does not forget the session's tools.
    asked(&mut engine, &user_message(3));
    asked(&mut engine, &call(4, "github_open_pr", ""));
    assert_eq!(
        asked(&mut engine, &turn_end(5, "", "")),
        ["rules/fix-ci/holds"]
    );
}

#[test]
fn integration_events_become_hooks_and_facts() {
    let transition = reminder(
        "transition",
        json!({ "history": "session", "source": ["jira"], "hooks": ["github:merged"] }),
    );
    let mut engine = engine(&[transition]);

    assert!(asked(&mut engine, &turn_end(1, "", "")).is_empty());

    // A Jira monitor on the ticket, then the PR (opened through the shell) merges.
    asked(&mut engine, &integration(2, "jira", "changelog"));
    asked(&mut engine, &integration(3, "github", "merged"));
    assert_eq!(
        asked(&mut engine, &turn_end(4, "", "")),
        ["rules/transition/holds"]
    );
}

#[test]
fn confirmed_rules_deliver_by_priority_up_to_the_limit() {
    let rules: Vec<Value> = [("low", 1), ("high", 9), ("mid", 5), ("absent", 20)]
        .into_iter()
        .map(|(id, priority)| with(reminder(id, json!({})), "priority", json!(priority)))
        .collect();
    let mut engine = engine(&rules);
    let ids = asked(&mut engine, &turn_end(1, "", ""));
    let answers = ids
        .iter()
        .map(|id| noul(id, if id.contains("absent") { 0.1 } else { 0.9 }))
        .collect();
    let step = engine.finish(Ok(answers), 1);

    assert_eq!(labels(&step), ["HIGH", "MID"]);
    assert_eq!(labels(&step).len(), MAX_DELIVERIES);
}

#[test]
fn a_step_needs_its_threshold() {
    let strict = reminder("strict", json!({}));
    let mut engine = engine(&[strict]);
    let ids = asked(&mut engine, &turn_end(1, "", ""));

    assert_eq!(answer_all(&mut engine, &ids, 0.65), Step::Done(vec![]));

    let ids = asked(&mut engine, &turn_end(2, "", ""));

    assert_eq!(labels(&answer_all(&mut engine, &ids, 0.7)), ["STRICT"]);
}

#[test]
fn once_lasts_its_history_window() {
    let session_once = with(
        reminder("session-once", json!({ "history": "session" })),
        "once",
        json!(true),
    );
    let turn_once = with(
        reminder("turn-once", json!({ "history": "turn" })),
        "once",
        json!(true),
    );
    let mut engine = engine(&[session_once, turn_once]);
    let ids = asked(&mut engine, &turn_end(1, "", ""));

    assert_eq!(
        labels(&answer_all(&mut engine, &ids, 0.9)),
        ["SESSION-ONCE", "TURN-ONCE"]
    );
    assert!(asked(&mut engine, &turn_end(2, "", "")).is_empty());

    // A new request renews the turn window's `once`, never the session's.
    asked(&mut engine, &user_message(3));
    assert_eq!(
        asked(&mut engine, &turn_end(4, "", "")),
        ["rules/turn-once/holds"]
    );
}

#[test]
fn cooldown_spaces_repeats() {
    let mut engine = engine(&[with(
        reminder("cool", json!({})),
        "cooldown_seconds",
        json!(60),
    )]);
    let ids = asked(&mut engine, &turn_end(10, "", ""));

    assert_eq!(labels(&answer_all(&mut engine, &ids, 0.9)), ["COOL"]);
    assert!(asked(&mut engine, &turn_end(20, "", "")).is_empty());
    assert_eq!(
        asked(&mut engine, &turn_end(70, "", "")),
        ["rules/cool/holds"]
    );
}

#[test]
fn memory_survives_a_save_and_load() {
    let rules = [reminder(
        "create-pr",
        json!({ "history": "session", "tools_called": ["edit"] }),
    )];
    let mut before = engine(&rules);

    asked(&mut before, &call(1, "edit", ""));

    let mut after = engine(&rules);

    after.load(before.save());
    assert_eq!(
        asked(&mut after, &turn_end(2, "", "")),
        ["rules/create-pr/holds"]
    );
}

// The format.

#[test]
fn rules_are_strict() {
    let ok = misuse("ok-id");
    let mut unknown = ok.clone();
    unknown["extra"] = json!(true);
    let mut below_even = ok.clone();
    below_even["steps"][0]["yes_at_or_above"] = json!(0.2);
    let unwatched = with(ok.clone(), "when", json!({}));
    let resume_on_call = with(
        ok.clone(),
        "then",
        json!({ "delivery": "resume", "text": "t" }),
    );
    let steer_at_turn_end = reminder("x", json!({})).pipe_then("steer");
    let watched_at_turn_end = reminder("x", json!({ "tools": ["shell"] }));
    let three_steps = rule("x", "turn_end", json!({}), &["a", "b", "c"], "resume");
    let old_schema = with(ok.clone(), "schema_version", json!(1));

    assert!(parsed(&ok).is_ok());
    for bad in [
        unknown,
        below_even,
        unwatched,
        resume_on_call,
        steer_at_turn_end,
        watched_at_turn_end,
        three_steps,
        old_schema,
        misuse("Bad ID"),
    ] {
        assert!(parsed(&bad).is_err(), "{bad}");
    }
}

trait PipeThen {
    fn pipe_then(self, delivery: &str) -> Value;
}

impl PipeThen for Value {
    fn pipe_then(mut self, delivery: &str) -> Value {
        self["then"]["delivery"] = json!(delivery);
        self
    }
}

#[test]
fn the_shipped_rules_load_and_hand_over_shipped_skills() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let rules = load_dir(&root.join("skills/rules")).expect("shipped rules are valid");

    assert!(rules.iter().any(|rule| rule.id == "hand-rolled-patch"));
    assert!(
        rules
            .iter()
            .any(|rule| rule.id == "jira-transition-after-merge")
    );
    for skill in rules.iter().filter_map(|rule| rule.then.skill.as_deref()) {
        assert!(root.join("skills/handoff").join(skill).is_dir(), "{skill}");
    }
}

// Project rules.

const PR: &str = r#"{
  "schema_version":2,"id":"hpdp-pr","name":"HPDP PR","on":"turn_end",
  "when":{"tools_called_any":["edit"]},
  "steps":[
    {"id":"intent","question":"Does the user want a PR?","yes_at_or_above":0.7,"minimum_confidence":0.4},
    {"id":"ready","question":"Are tests complete?","yes_at_or_above":0.7,"minimum_confidence":0.4}
  ],
  "then":{"delivery":"resume","text":"Open the PR after checks."},"once":true
}"#;

const CHANGELOG: &str = r#"{
  "schema_version":2,"id":"changelog","name":"Changelog","on":"turn_end",
  "steps":[
    {"id":"notable","question":"Is the change notable?","yes_at_or_above":0.7,"minimum_confidence":0.4}
  ],
  "then":{"delivery":"wait","text":"Add a changelog entry."}
}"#;

/// Tests run in parallel threads; each workspace gets its own directory.
static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chauffeur-rules-{}-{}",
            std::process::id(),
            NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed)
        ));

        // A directory left by an earlier run with the same process ID.
        let _ = std::fs::remove_dir_all(&path);

        std::fs::create_dir_all(path.join("project/.git")).unwrap();
        std::fs::create_dir_all(path.join("project/.chauffeur/rules")).unwrap();
        std::fs::create_dir_all(path.join("sibling/.git")).unwrap();
        std::fs::write(path.join("project/.chauffeur/rules/pr.json"), PR).unwrap();

        Self(path)
    }

    fn project(&self) -> String {
        self.0.join("project").to_string_lossy().into_owned()
    }

    fn sibling(&self) -> String {
        self.0.join("sibling").to_string_lossy().into_owned()
    }

    fn rule(&self, name: &str) -> PathBuf {
        self.0.join("project/.chauffeur/rules").join(name)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Confirm the intent step and return the readiness question's ID.
fn confirm_intent(engine: &mut Engine, turn: &Signal) -> String {
    let ids = asked(engine, turn);
    let Step::Ask { questions, .. } = answer_all(engine, &ids, 0.95) else {
        panic!("expected readiness")
    };

    questions[0].id.clone()
}

#[test]
fn a_sibling_repository_contributes_no_project_rules() {
    let workspace = Workspace::new();
    let mut engine = engine(&[]);

    asked(&mut engine, &call(1, "edit", &workspace.project()));

    assert!(load_project(&workspace.sibling()).unwrap().is_empty());
    assert!(asked(&mut engine, &turn_end(2, &workspace.sibling(), "open a PR")).is_empty());
}

#[test]
fn edits_in_another_workspace_do_not_count_for_this_one() {
    let workspace = Workspace::new();
    let mut engine = engine(&[]);

    asked(&mut engine, &call(1, "edit", &workspace.sibling()));

    assert!(asked(&mut engine, &turn_end(2, &workspace.project(), "open a PR")).is_empty());
}

#[test]
fn an_explicit_user_opt_out_ends_the_rule_at_its_first_step() {
    let workspace = Workspace::new();
    let mut engine = engine(&[]);

    asked(&mut engine, &call(1, "edit", &workspace.project()));

    let Step::Ask { questions, .. } = engine
        .begin(&turn_end(2, &workspace.project(), "do not create a PR"))
        .unwrap()
    else {
        panic!("expected intent")
    };

    assert!(questions[0].instructions.contains("do not create a PR"));
    assert_eq!(
        engine.finish(Ok(vec![noul(&questions[0].id, 0.05)]), 1),
        Step::Done(vec![])
    );
}

#[test]
fn a_confirmed_project_rule_delivers_once_until_the_next_user_message() {
    let workspace = Workspace::new();
    let mut engine = engine(&[]);
    let edit = call(1, "edit", &workspace.project());
    let pr = turn_end(2, &workspace.project(), "please open a PR once verified");

    asked(&mut engine, &edit);

    let ready = confirm_intent(&mut engine, &pr);
    assert_eq!(
        engine.finish(Ok(vec![noul(&ready, 0.2)]), 1),
        Step::Done(vec![])
    );

    let ready = confirm_intent(&mut engine, &pr);
    assert_eq!(ready, "rules/hpdp-pr/ready");
    let Step::Done(effects) = engine.finish(Ok(vec![noul(&ready, 0.95)]), 1) else {
        panic!("expected effect")
    };
    assert!(
        matches!(effects.as_slice(), [Effect::Context { delivery: Delivery::Resume, text: Some(text), skills, .. }] if text.contains("Open the PR") && skills.is_empty())
    );

    let mut restarted = self::engine(&[]);
    restarted.load(engine.save());
    assert!(asked(&mut restarted, &pr).is_empty());

    // A new request needs a new edit before the rule applies again.
    asked(&mut restarted, &user_message(3));
    assert!(asked(&mut restarted, &pr).is_empty());
    asked(&mut restarted, &edit);
    assert_eq!(asked(&mut restarted, &pr), ["rules/hpdp-pr/intent"]);
}

#[test]
fn a_one_step_rule_still_delivers_when_another_moves_to_its_second_step() {
    let workspace = Workspace::new();
    std::fs::write(workspace.rule("changelog.json"), CHANGELOG).unwrap();
    let mut engine = engine(&[]);

    asked(&mut engine, &call(1, "edit", &workspace.project()));

    let ids = asked(&mut engine, &turn_end(2, &workspace.project(), ""));
    assert_eq!(ids.len(), 2);
    let Step::Ask { questions, .. } = answer_all(&mut engine, &ids, 0.95) else {
        panic!("expected readiness")
    };
    let ready: Vec<String> = questions.into_iter().map(|question| question.id).collect();

    // Equal priorities deliver in the order they were confirmed.
    assert_eq!(
        labels(&answer_all(&mut engine, &ready, 0.95)),
        ["Changelog", "HPDP PR"]
    );

    // A failed second round still delivers what round one confirmed.
    let mut engine = self::engine(&[]);

    asked(&mut engine, &call(1, "edit", &workspace.project()));

    let ids = asked(&mut engine, &turn_end(2, &workspace.project(), ""));
    let Step::Ask { .. } = answer_all(&mut engine, &ids, 0.95) else {
        panic!("expected readiness")
    };

    assert_eq!(labels(&engine.finish(Err("down".into()), 1)), ["Changelog"]);
}

#[test]
fn a_project_rule_cannot_replace_a_shipped_one() {
    let workspace = Workspace::new();
    let shipped = reminder("hpdp-pr", json!({ "tools_called_any": ["edit"] }));
    let mut engine = engine(&[shipped]);

    asked(&mut engine, &call(1, "edit", &workspace.project()));
    assert_eq!(
        asked(&mut engine, &turn_end(2, &workspace.project(), "")),
        ["rules/hpdp-pr/holds"]
    );
}

#[test]
fn invalid_duplicate_and_symlinked_project_rules_are_rejected() {
    let workspace = Workspace::new();
    let project = workspace.project();

    std::fs::write(
        workspace.rule("pr.json"),
        PR.replace("\"once\":true", "\"once\":true,\"unknown\":0"),
    )
    .unwrap();
    assert!(load_project(&project).is_err());

    std::fs::write(workspace.rule("pr.json"), PR).unwrap();
    std::fs::write(workspace.rule("duplicate.json"), PR).unwrap();
    assert!(load_project(&project).is_err());

    std::fs::remove_file(workspace.rule("duplicate.json")).unwrap();

    #[cfg(unix)]
    {
        let outside = workspace.0.join("outside.json");

        std::fs::write(&outside, CHANGELOG).unwrap();
        std::os::unix::fs::symlink(&outside, workspace.rule("outside.json")).unwrap();
        assert!(load_project(&project).is_err());
    }
}
