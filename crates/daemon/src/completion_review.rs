//! Labeled completion replay through the shipped profile; no live daemon or config.

use std::path::PathBuf;
use std::time::Instant;

use chauffeur_capability_rules::{Rulebook, Rules};
use chauffeur_core::{
    Answer, AnswerValue, Effect, Engine, Judging, Reasoned, Reasoning, Rule, RulebookCommand,
    Signal, SignalKind, Step, SystemOne, SystemTwo, Todo, TodoState,
};
use chauffeur_judge_jev::{JevClient, JevConfig};
use serde::Deserialize;
use serde_json::{Value, json};

const CASES: &str = include_str!("../../../bench/judges/completion-review.json");
const GOAL: &str = include_str!("../../../skills/rulebooks/goal.json");

#[derive(Deserialize)]
struct Case {
    id: String,
    goal: String,
    summary: String,
    activity: String,
    todos: Vec<Todo>,
    label: String,
}

fn signal(kind: SignalKind) -> Signal {
    Signal {
        agent_id: "evaluation".into(),
        at: 1,
        kind,
    }
}

fn prepare(case: &Case) -> (Engine, Step, Rule) {
    let mut book = Rulebook::from_json(GOAL.as_bytes()).unwrap();
    book.rules.retain(|rule| rule.id == "goal-done");
    let step = &mut book.rules[0].steps[0];
    step.review.as_mut().unwrap().enabled = true;
    let threshold = Rule::yes(step.yes_at_or_above, step.minimum_confidence);
    let mut engine = Engine::hosted(vec![Box::new(Judging::new(
        Rules::new(Vec::new()).with_rulebooks(vec![book], true),
    ))])
    .unwrap();
    assert!(matches!(
        engine
            .begin(&signal(SignalKind::Rulebook {
                command: RulebookCommand::Start,
                rulebook: "goal".into(),
                args: case.goal.clone(),
                workspace: String::new(),
            }))
            .unwrap(),
        Step::Done(_)
    ));
    if !case.activity.is_empty() {
        assert!(matches!(
            engine
                .begin(&signal(SignalKind::ToolResult {
                    tool: "shell".into(),
                    ok: true,
                    input: "verification".into(),
                    evidence: case.activity.clone(),
                    error: String::new(),
                    workspace: String::new(),
                    subagent: false,
                    user_request: case.goal.clone(),
                    candidates: Vec::new(),
                }))
                .unwrap(),
            Step::Done(_)
        ));
    }
    let step = engine
        .begin(&signal(SignalKind::TurnEnd {
            workspace: String::new(),
            subagent: false,
            user_request: case.goal.clone(),
            summary: case.summary.clone(),
            todos: case.todos.clone(),
        }))
        .unwrap();
    (engine, step, threshold)
}

fn direct_request(case: &Case) -> Option<Reasoning> {
    let (mut engine, step, _) = prepare(case);
    let Step::Ask { questions, .. } = step else {
        return None;
    };
    let unsure = questions
        .iter()
        .map(|question| Answer {
            id: question.id.clone(),
            value: AnswerValue::Noul(0.5),
            confidence: None,
        })
        .collect();
    let Step::Ask {
        mut reasonings,
        questions,
        ..
    } = engine.finish(Ok(unsure), 0)
    else {
        panic!("expected a review")
    };
    assert!(questions.is_empty());
    assert_eq!(reasonings.len(), 1);
    reasonings.pop()
}

fn complete(effects: &[Effect]) -> bool {
    effects.iter().any(|effect| {
        matches!(effect, Effect::Context { text: Some(text), .. }
        if text.starts_with("Goal achieved:"))
    })
}

fn millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn outcome(case: &Case, mode: &str, verdict: Value, holds: bool, elapsed_ms: u64) -> Value {
    json!({
        "case": case.id, "mode": mode, "label": case.label, "verdict": verdict,
        "complete": holds, "agreement": holds == (case.label == "complete"),
        "elapsed_ms": elapsed_ms,
    })
}

fn screened(case: &Case, one: &mut dyn SystemOne, two: &mut dyn SystemTwo) -> Vec<Value> {
    let (mut engine, step, threshold) = prepare(case);
    let Step::Ask {
        state, questions, ..
    } = step
    else {
        return ["system_one", "screen_review"]
            .map(|mode| outcome(case, mode, json!("todo_veto"), false, 0))
            .to_vec();
    };
    let started = Instant::now();
    let answers = one.ask(&state, &questions).map_err(|error| error.0);
    let elapsed = millis(started);
    let answer = answers.as_ref().ok().and_then(|answers| answers.first());
    let held = threshold.holds(answer);
    let first = outcome(
        case,
        "system_one",
        json!({
            "held": held, "answer": format!("{answer:?}"),
        }),
        held,
        elapsed,
    );
    let step = engine.finish(answers, elapsed);
    let (effects, verdict) = match step {
        Step::Done(effects) => (effects, json!(answer_label(held))),
        Step::Ask {
            questions,
            reasonings,
            ..
        } => {
            assert!(questions.is_empty());
            let replies: Vec<(String, Reasoned)> = reasonings
                .iter()
                .map(|request| {
                    (
                        request.id.clone(),
                        chauffeur_core::system_two::run(Some(two), request),
                    )
                })
                .collect();
            let verdict = replies
                .first()
                .map(|(_, reply)| reply.clone().unwrap_or_else(|failure| json!(failure)))
                .unwrap_or(Value::Null);
            let Step::Done(effects) = engine.finish_round(Ok(Vec::new()), replies, 0) else {
                panic!("exceeded two rounds")
            };
            (effects, verdict)
        }
    };
    vec![
        first,
        outcome(
            case,
            "screen_review",
            verdict,
            complete(&effects),
            millis(started),
        ),
    ]
}

fn answer_label(held: bool) -> &'static str {
    if held { "screen_yes" } else { "screen_not_yes" }
}

fn direct(case: &Case, two: &mut dyn SystemTwo) -> Value {
    let Some(request) = direct_request(case) else {
        return outcome(case, "direct_review", json!("todo_veto"), false, 0);
    };
    let started = Instant::now();
    let reply: Reasoned = chauffeur_core::system_two::run(Some(two), &request);
    let held = reply.as_ref().is_ok_and(|value| {
        value["verdict"] == "complete"
            && value["cites"]
                .as_array()
                .is_some_and(|cites| !cites.is_empty())
            && value["reason"]
                .as_str()
                .is_some_and(|reason| !reason.trim().is_empty())
    });
    let verdict = reply.unwrap_or_else(|failure| json!(failure));
    outcome(case, "direct_review", verdict, held, millis(started))
}

struct ConfigFolder(PathBuf);

impl ConfigFolder {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("completion-review-eval-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(
            path.join("judge.json"),
            r#"{"system_two":{"enabled":true}}"#,
        )
        .unwrap();
        Self(path)
    }
}

impl Drop for ConfigFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("judge.json"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

#[test]
#[ignore = "live TypeSafe + Codex calls; may refresh the Codex login"]
fn completion_review_live() {
    let mut one = JevClient::new(JevConfig::from_env().expect("set TYPESAFE_API_KEY")).unwrap();
    let config = ConfigFolder::new();
    let mut two = crate::reasoning::factory(&config.0).unwrap()().unwrap();
    let cases: Vec<Case> = serde_json::from_str(CASES).unwrap();
    let mut results = Vec::new();
    for case in &cases {
        results.extend(screened(case, &mut one, two.as_mut()));
        results.push(direct(case, two.as_mut()));
    }
    for result in &results {
        println!("{}", serde_json::to_string(result).unwrap());
    }
    for mode in ["system_one", "screen_review", "direct_review"] {
        let rows: Vec<&Value> = results.iter().filter(|row| row["mode"] == mode).collect();
        let agreed = rows.iter().filter(|row| row["agreement"] == true).count();
        let mut latencies: Vec<u64> = rows
            .iter()
            .map(|row| row["elapsed_ms"].as_u64().unwrap())
            .collect();
        latencies.sort_unstable();
        println!(
            "{mode}: {agreed}/{} completion agreement; p50={}ms p95={}ms",
            rows.len(),
            latencies[(latencies.len() * 50).div_ceil(100) - 1],
            latencies[(latencies.len() * 95).div_ceil(100) - 1]
        );
    }
}

#[test]
fn completion_review_cases_have_labels_and_use_the_real_rule_evidence() {
    let cases: Vec<Case> = serde_json::from_str(CASES).unwrap();
    assert_eq!(cases.len(), 16);
    let mut ids = std::collections::HashSet::new();
    for case in &cases {
        assert!(ids.insert(&case.id));
        assert!(
            ["complete", "continue", "blocked", "insufficient_evidence"]
                .contains(&case.label.as_str())
        );
        let request = direct_request(case);
        if TodoState::of(&case.todos) == TodoState::Open {
            assert!(request.is_none());
        } else {
            let request = request.unwrap();
            assert_eq!(request.profile, "completion-review");
            assert!(request.instructions.contains(&case.goal));
            assert!(request.evidence.iter().any(|piece| piece.id == "activity"));
        }
    }
}

#[test]
fn completion_review_replay_exercises_all_three_paths_without_services() {
    struct Unsure;
    impl SystemOne for Unsure {
        fn name(&self) -> &str {
            "unsure"
        }
        fn ask(
            &mut self,
            _: &str,
            questions: &[chauffeur_core::Question],
        ) -> Result<Vec<Answer>, chauffeur_core::SystemOneError> {
            Ok(questions
                .iter()
                .map(|question| Answer {
                    id: question.id.clone(),
                    value: AnswerValue::Noul(0.5),
                    confidence: None,
                })
                .collect())
        }
    }
    struct Cited(usize);
    impl SystemTwo for Cited {
        fn name(&self) -> &str {
            "cited"
        }
        fn reason(&mut self, _: &Reasoning) -> Reasoned {
            self.0 += 1;
            Ok(json!({"verdict":"complete", "cites":["todos"], "reason":"fixture"}))
        }
    }
    let cases: Vec<Case> = serde_json::from_str(CASES).unwrap();
    let mut two = Cited(0);
    for case in &cases {
        let holds = TodoState::of(&case.todos) != TodoState::Open;
        let rows = screened(case, &mut Unsure, &mut two);
        assert_eq!(rows[0]["complete"], false);
        assert_eq!(rows[1]["complete"], holds);
        assert_eq!(direct(case, &mut two)["complete"], holds);
    }
    assert_eq!(two.0, 30);
}
