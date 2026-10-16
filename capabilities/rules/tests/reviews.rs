//! Opt-in rule reviews through real engine rounds and verdict validation.

use std::sync::{Arc, Mutex};

use chauffeur_capability_rules::{Rulebook, Rules};
use chauffeur_core::{
    Answer, AnswerValue, Effect, Engine, Judging, Question, Reasoned, Reasoning, ReasoningFailure,
    RulebookCommand, Signal, SignalKind, SystemOne, SystemOneError, SystemTwo, Todo, TodoStatus,
};
use serde_json::{Value, json};

const GOAL: &str = include_str!("../../../skills/rulebooks/goal.json");

fn book(enabled: bool, failed: bool) -> Rulebook {
    let mut value: Value = serde_json::from_str(GOAL).unwrap();
    value["rules"][0]["steps"][0]["review"]["enabled"] = json!(enabled);
    value["rules"][0]["steps"][0]["review"]["when"]["failed"] = json!(failed);
    Rulebook::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
}

struct One {
    done: Result<f32, ()>,
    confidence: Option<f32>,
    other: f32,
}

impl SystemOne for One {
    fn name(&self) -> &str {
        "fixed"
    }

    fn ask(&mut self, _: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        let done = self.done.map_err(|()| SystemOneError("offline".into()))?;
        Ok(questions
            .iter()
            .map(|question| Answer {
                id: question.id.clone(),
                value: AnswerValue::Noul(if question.id.contains("goal-done") {
                    done
                } else {
                    self.other
                }),
                confidence: self.confidence,
            })
            .collect())
    }
}

struct Two {
    reply: Reasoned,
    seen: Arc<Mutex<Vec<Reasoning>>>,
}

impl SystemTwo for Two {
    fn name(&self) -> &str {
        "review"
    }

    fn reason(&mut self, request: &Reasoning) -> Reasoned {
        self.seen.lock().unwrap().push(request.clone());
        self.reply.clone()
    }
}

fn engine(
    book: Rulebook,
    one: One,
    reply: Option<Reasoned>,
) -> (Engine, Arc<Mutex<Vec<Reasoning>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(
        Box::new(one),
        vec![Box::new(Judging::new(
            Rules::new(Vec::new()).with_rulebooks(vec![book], true),
        ))],
    )
    .unwrap();
    if let Some(reply) = reply {
        engine = engine.with_system_two(Box::new(Two {
            reply,
            seen: Arc::clone(&seen),
        }));
    }
    engine
        .ingest(&signal(SignalKind::Rulebook {
            command: RulebookCommand::Start,
            rulebook: "goal".into(),
            args: "ship the release".into(),
            workspace: String::new(),
        }))
        .unwrap();
    (engine, seen)
}

fn one(done: Result<f32, ()>) -> One {
    One {
        done,
        confidence: None,
        other: 0.1,
    }
}

fn signal(kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind,
    }
}

fn turn(todos: Vec<Todo>) -> Signal {
    signal(SignalKind::TurnEnd {
        workspace: String::new(),
        subagent: false,
        user_request: "Release the artifact".into(),
        summary: "Release uploaded; smoke test: passed".into(),
        todos,
    })
}

fn verdict(verdict: &str, cites: &[&str]) -> Reasoned {
    Ok(json!({ "verdict": verdict, "cites": cites, "reason": "Observed checks" }))
}

fn complete() -> Reasoned {
    verdict("complete", &["summary"])
}

fn completed(effects: &[Effect]) -> bool {
    effects.iter().any(|effect| {
        matches!(effect, Effect::Context { text: Some(text), .. }
        if text.starts_with("Goal achieved:"))
    })
}

#[test]
fn a_confident_yes_completes_without_review_and_a_confident_no_never_escalates() {
    for (p, holds) in [(0.95, true), (0.05, false)] {
        let (mut engine, seen) = engine(book(true, true), one(Ok(p)), Some(complete()));
        assert_eq!(completed(&engine.ingest(&turn(vec![])).unwrap()), holds);
        assert!(seen.lock().unwrap().is_empty());
    }
}

#[test]
fn an_unsure_screen_completes_only_on_a_cited_complete_verdict() {
    let (mut engine, seen) = engine(book(true, true), one(Ok(0.5)), Some(complete()));
    assert!(completed(&engine.ingest(&turn(vec![])).unwrap()));
    let seen = seen.lock().unwrap();
    let [request] = seen.as_slice() else {
        panic!("expected one review")
    };
    assert_eq!(request.id, "rules/goal-done/done/review");
    assert_eq!(request.profile, "completion-review");
    assert!(request.instructions.contains("ship the release"));
    assert!(!request.instructions.contains("{args}"));
    assert!(request.activity);
    let ids: Vec<&str> = request
        .evidence
        .iter()
        .map(|piece| piece.id.as_str())
        .collect();
    assert_eq!(ids, ["activity", "user_request", "summary", "todos"]);
    assert!(request.evidence[1].text.contains("Release the artifact"));
    assert!(request.evidence[2].text.contains("smoke test: passed"));
    assert_eq!(request.evidence[3].text, "[]");
    assert_eq!(engine.trace().reasonings[0].round, 2);
}

#[test]
fn missing_uncertain_failed_or_invalid_reviews_never_complete() {
    let replies = [
        None,
        Some(verdict("continue", &["summary"])),
        Some(verdict("blocked", &["summary"])),
        Some(verdict("insufficient_evidence", &[])),
        Some(Err(ReasoningFailure::Unavailable("off".into()))),
        Some(Err(ReasoningFailure::Refused)),
        Some(Err(ReasoningFailure::TimedOut)),
        Some(Err(ReasoningFailure::Failed("offline".into()))),
        Some(Err(ReasoningFailure::Truncated)),
        Some(verdict("complete", &["invented"])),
        Some(verdict("complete", &[])),
        Some(Ok(json!({ "verdict": "complete", "cites": ["summary"] }))),
        Some(Ok(
            json!({ "verdict": "complete", "cites": ["summary"], "reason": " " }),
        )),
    ];
    for reply in replies {
        let (mut engine, _) = engine(book(true, true), one(Ok(0.5)), reply.clone());
        assert!(
            !completed(&engine.ingest(&turn(vec![])).unwrap()),
            "{reply:?}"
        );
        assert_eq!(engine.trace().reasonings.len(), 1);
    }
}

#[test]
fn a_failed_screen_uses_only_its_configured_failed_policy() {
    for failed in [false, true] {
        let (mut engine, seen) = engine(book(true, failed), one(Err(())), Some(complete()));
        assert_eq!(completed(&engine.ingest(&turn(vec![])).unwrap()), failed);
        assert_eq!(seen.lock().unwrap().len(), usize::from(failed));
    }
}

#[test]
fn a_disabled_review_preserves_every_system_one_outcome_even_with_a_live_executor() {
    for (answer, holds) in [
        (Ok(0.95), true),
        (Ok(0.5), false),
        (Ok(0.05), false),
        (Err(()), false),
    ] {
        let (mut engine, seen) = engine(book(false, true), one(answer), Some(complete()));
        assert_eq!(completed(&engine.ingest(&turn(vec![])).unwrap()), holds);
        assert!(seen.lock().unwrap().is_empty());
        assert!(engine.trace().reasonings.is_empty());
    }
    assert!(
        !Rulebook::from_json(GOAL.as_bytes()).unwrap().rules[0].steps[0]
            .review
            .as_ref()
            .unwrap()
            .enabled
    );
}

#[test]
fn open_todos_veto_review_and_completion_regardless_of_the_report() {
    for status in [TodoStatus::Pending, TodoStatus::InProgress] {
        let (mut engine, seen) = engine(book(true, true), one(Ok(0.5)), Some(complete()));
        let turn = turn(vec![Todo {
            content: "verify deployment".into(),
            status,
        }]);
        assert!(!completed(&engine.ingest(&turn).unwrap()));
        assert!(seen.lock().unwrap().is_empty());
        assert!(
            engine
                .trace()
                .questions
                .iter()
                .all(|q| !q.id.contains("goal-done"))
        );
    }
}

#[test]
fn low_confidence_is_unsure_even_when_probability_is_high() {
    let mut one = one(Ok(0.95));
    one.confidence = Some(0.1);
    let (mut engine, seen) = engine(book(true, true), one, Some(complete()));
    assert!(completed(&engine.ingest(&turn(vec![])).unwrap()));
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[test]
fn a_review_keeps_existing_pause_and_continuation_priorities() {
    for (reply, expected) in [
        (complete(), "Goal achieved:"),
        (
            verdict("insufficient_evidence", &[]),
            "Goal paused until you answer",
        ),
    ] {
        let mut one = one(Ok(0.5));
        one.other = 0.95;
        let (mut engine, _) = engine(book(true, true), one, Some(reply));
        let effects = engine.ingest(&turn(vec![])).unwrap();
        assert_eq!(effects.len(), 1);
        assert!(
            matches!(&effects[0], Effect::Context {text: Some(text), ..} if text.starts_with(expected))
        );
    }
    let mut one = one(Ok(0.5));
    one.other = 0.95;
    let mut book = book(true, true);
    book.rules
        .retain(|rule| rule.id != "goal-blocked" && rule.id != "goal-todos");
    let (mut engine, _) = engine(book, one, Some(verdict("continue", &["summary"])));
    let effects = engine.ingest(&turn(vec![])).unwrap();
    assert!(
        matches!(&effects[0], Effect::Context {text: Some(text), ..} if text.contains("not yet achieved"))
    );
}

#[test]
fn review_configuration_is_bounded_strict_and_cannot_spend_three_rounds() {
    let base: Value = serde_json::from_str(GOAL).unwrap();
    let mut chain = base.clone();
    let mut second = chain["rules"][0]["steps"][0].clone();
    second["id"] = json!("extra");
    second.as_object_mut().unwrap().remove("review");
    chain["rules"][0]["steps"]
        .as_array_mut()
        .unwrap()
        .push(second);
    assert!(
        Rulebook::from_json(&serde_json::to_vec(&chain).unwrap())
            .unwrap_err()
            .contains("2-round budget")
    );
    for (field, value) in [
        ("profile", json!("")),
        ("instructions", json!("{unknown}")),
        ("instructions", json!("x".repeat(4097))),
        ("holds_on", json!("insufficient_evidence")),
        ("holds_on", json!("continue")),
        ("when", json!({})),
        ("unexpected", json!(true)),
    ] {
        let mut bad = base.clone();
        bad["rules"][0]["steps"][0]["review"][field] = value;
        assert!(
            Rulebook::from_json(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn review_selection_and_holding_verdict_are_rule_owned() {
    let mut selected = book(true, true);
    selected.rules[0].steps[0]
        .review
        .as_mut()
        .unwrap()
        .when
        .unsure = false;
    let (mut engine, seen) = engine(selected, one(Ok(0.5)), Some(complete()));
    assert!(!completed(&engine.ingest(&turn(vec![])).unwrap()));
    assert!(seen.lock().unwrap().is_empty());

    for verdict_name in ["continue", "blocked"] {
        let mut value: Value = serde_json::from_str(GOAL).unwrap();
        value["rules"][0]["then"]
            .as_object_mut()
            .unwrap()
            .remove("end");
        value["rules"][0]["then"]["text"] = json!("Review holds");
        value["rules"][0]["steps"][0]["review"]["enabled"] = json!(true);
        value["rules"][0]["steps"][0]["review"]["holds_on"] = json!(verdict_name);
        let book = Rulebook::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
        let (mut engine, _) = self::engine(
            book,
            one(Ok(0.5)),
            Some(verdict(verdict_name, &["summary"])),
        );
        let effects = engine.ingest(&turn(vec![])).unwrap();
        assert!(
            matches!(&effects[0], Effect::Context {text: Some(text), ..} if text == "Review holds")
        );
    }
}

#[test]
fn review_is_turn_end_only_and_an_omitted_flag_is_off() {
    let mut value: Value = serde_json::from_str(GOAL).unwrap();
    value["rules"][0]["steps"][0]["review"]
        .as_object_mut()
        .unwrap()
        .remove("enabled");
    let parsed = Rulebook::from_json(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(!parsed.rules[0].steps[0].review.as_ref().unwrap().enabled);
    value["rules"][0]["on"] = json!("tool_result");
    value["rules"][0]["when"] = json!({ "tools": ["shell"] });
    value["rules"][0]["then"] = json!({ "delivery": "steer", "text": "reviewed" });
    let error = Rulebook::from_json(&serde_json::to_vec(&value).unwrap()).unwrap_err();
    assert!(error.contains("review applies only to turn_end"), "{error}");
}
