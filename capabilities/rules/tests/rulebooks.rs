//! Rulebooks through the engine: the shipped goal, starting and stopping a
//! book per session, one delivery per book, the no-spin guard, the budget,
//! and the evidence a turn-end question carries.

use std::path::Path;

use chauffeur_capability_rules::{Rule, Rulebook, Rules, load_rulebooks};
use chauffeur_core::{
    Answer, AnswerValue, Delivery, Effect, Engine, Judging, RulebookCommand, Signal, SignalKind,
    Step, Todo, TodoStatus,
};
use serde_json::{Value, json};

fn shipped() -> Vec<Rulebook> {
    load_rulebooks(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/rulebooks")).unwrap()
}

fn engine(books: Vec<Rulebook>) -> Engine {
    let rules = Rules::new(Vec::new()).with_rulebooks(books, true);

    Engine::hosted(vec![Box::new(Judging::new(rules))]).unwrap()
}

fn signal(at: u64, kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at,
        kind,
    }
}

fn command(at: u64, command: RulebookCommand, args: &str) -> Signal {
    signal(
        at,
        SignalKind::Rulebook {
            command,
            rulebook: "goal".into(),
            args: args.into(),
            workspace: String::new(),
        },
    )
}

fn turn_end(at: u64, summary: &str) -> Signal {
    turn_end_with(at, summary, Vec::new())
}

fn turn_end_with(at: u64, summary: &str, todos: Vec<Todo>) -> Signal {
    signal(
        at,
        SignalKind::TurnEnd {
            workspace: String::new(),
            subagent: false,
            user_request: "make the tests pass".into(),
            summary: summary.into(),
            todos,
        },
    )
}

fn tool(at: u64) -> Signal {
    signal(
        at,
        SignalKind::ToolResult {
            tool: "shell".into(),
            ok: true,
            workspace: String::new(),
            subagent: false,
            input: "{}".into(),
            error: String::new(),
            user_request: String::new(),
            evidence: String::new(),
            candidates: Vec::new(),
        },
    )
}

/// Effects of a signal that settles without asking.
fn settled(engine: &mut Engine, signal: &Signal) -> Vec<(Delivery, String)> {
    match engine.begin(signal).unwrap() {
        Step::Done(effects) => contexts(&effects),
        Step::Ask { questions, .. } => panic!("asked {questions:?}"),
    }
}

/// The questions a signal asks, as (id, instructions).
fn asked(engine: &mut Engine, signal: &Signal) -> Vec<(String, String)> {
    match engine.begin(signal).unwrap() {
        Step::Ask { questions, .. } => questions
            .into_iter()
            .map(|question| (question.id, question.instructions))
            .collect(),
        Step::Done(effects) => {
            assert!(effects.is_empty(), "settled with {effects:?}");
            Vec::new()
        }
    }
}

/// Answer each asked question: `yes` names the ones answered 0.9.
fn answer(engine: &mut Engine, ids: &[(String, String)], yes: &[&str]) -> Vec<(Delivery, String)> {
    let answers = ids
        .iter()
        .map(|(id, _)| Answer {
            id: id.clone(),
            value: AnswerValue::Noul(if yes.contains(&id.as_str()) { 0.9 } else { 0.1 }),
            confidence: None,
        })
        .collect();

    match engine.finish(Ok(answers), 1) {
        Step::Done(effects) => contexts(&effects),
        Step::Ask { questions, .. } => panic!("asked again {questions:?}"),
    }
}

fn contexts(effects: &[Effect]) -> Vec<(Delivery, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Context { delivery, text, .. } => {
                Some((*delivery, text.clone().unwrap_or_default()))
            }
            _ => None,
        })
        .collect()
}

const DONE: &str = "rules/goal-done/done";
const BLOCKED: &str = "rules/goal-blocked/needs-user";
const TODOS: &str = "rules/goal-todos/several-steps";

fn todo(content: &str, status: TodoStatus) -> Todo {
    Todo {
        content: content.into(),
        status,
    }
}
const CONTINUE: &str = "rules/goal-continue/not-done";

#[test]
fn a_goal_continues_until_evidence_says_done() {
    let mut engine = engine(shipped());

    assert!(
        asked(&mut engine, &turn_end(1, "")).is_empty(),
        "no book, no questions"
    );

    let started = settled(
        &mut engine,
        &command(2, RulebookCommand::Start, "p95 under 120 ms"),
    );
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].0, Delivery::Resume);
    assert!(
        started[0].1.contains("Goal: p95 under 120 ms"),
        "{started:?}"
    );

    let questions = asked(
        &mut engine,
        &turn_end(3, "Profiled the handler; p95 is 180 ms."),
    );
    // No todos yet: whether the goal needs them is asked too.
    let ids: Vec<&str> = questions.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    for id in [DONE, BLOCKED, TODOS] {
        assert!(ids.contains(&id), "{ids:?}");
    }
    assert!(
        questions
            .iter()
            .all(|(_, text)| text.contains("p95 under 120 ms"))
    );
    assert!(
        questions[0]
            .1
            .contains("closing message this turn: Profiled the handler")
    );

    // Neither done nor blocked: the book's `otherwise` keeps the agent going.
    let delivered = answer(&mut engine, &questions, &[]);
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].0, Delivery::Resume);
    assert!(delivered[0].1.contains("Not done yet"));

    // The continuation turn did nothing: the book stays quiet.
    assert!(asked(&mut engine, &turn_end(4, "")).is_empty());

    assert!(asked(&mut engine, &tool(5)).is_empty());
    let questions = asked(
        &mut engine,
        &turn_end(6, "p95 is 104 ms; benchmark attached."),
    );
    assert!(
        questions[0]
            .1
            .contains("Tools it called since that request: shell."),
        "{questions:?}"
    );

    // Done and blocked both hold: only the higher-priority "done" delivers.
    let delivered = answer(&mut engine, &questions, &[DONE, BLOCKED]);
    assert_eq!(
        delivered,
        vec![(
            Delivery::Wait,
            "Goal achieved: p95 under 120 ms".to_string()
        )]
    );

    assert!(asked(&mut engine, &tool(7)).is_empty());
    assert!(
        asked(&mut engine, &turn_end(8, "")).is_empty(),
        "a completed goal is gone"
    );
}

#[test]
fn a_blocked_goal_pauses_until_resumed() {
    let mut engine = engine(shipped());

    settled(&mut engine, &command(1, RulebookCommand::Start, "ship it"));
    let questions = asked(&mut engine, &turn_end(2, "I need the staging password."));
    let delivered = answer(&mut engine, &questions, &[BLOCKED]);
    assert!(delivered[0].1.contains("Goal paused"), "{delivered:?}");

    assert!(asked(&mut engine, &tool(3)).is_empty());
    assert!(asked(&mut engine, &turn_end(4, "")).is_empty(), "paused");

    let status = settled(&mut engine, &command(5, RulebookCommand::Status, ""));
    assert!(
        status[0].1.contains("Goal is paused: ship it"),
        "{status:?}"
    );

    let resumed = settled(&mut engine, &command(6, RulebookCommand::Resume, ""));
    assert_eq!(resumed[0].0, Delivery::Resume);
    assert_eq!(asked(&mut engine, &turn_end(7, "")).len(), 3);
}

#[test]
fn a_goal_with_open_todos_cannot_finish_and_lists_them() {
    let mut engine = engine(shipped());

    let started = settled(&mut engine, &command(1, RulebookCommand::Start, "ship it"));
    assert!(started[0].1.contains("todowrite"), "{started:?}");

    // Open todos: "done" is not even asked, and the nudge names what is open.
    let open = vec![
        todo("write the migration", TodoStatus::Completed),
        todo("backfill old rows", TodoStatus::InProgress),
        todo("update the docs", TodoStatus::Pending),
    ];
    let questions = asked(&mut engine, &turn_end_with(2, "Migration written.", open));
    let ids: Vec<&str> = questions.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, vec![BLOCKED], "{ids:?}");

    // Even a report that says done keeps going while todos are open.
    let delivered = answer(&mut engine, &questions, &[]);
    assert_eq!(delivered[0].0, Delivery::Resume);
    assert!(
        delivered[0]
            .1
            .contains("Open todos:\n- backfill old rows (in progress)\n- update the docs\n"),
        "{delivered:?}"
    );
    assert!(!delivered[0].1.contains("write the migration"));

    // Every todo completed or cancelled: "done" is asked again.
    assert!(asked(&mut engine, &tool(3)).is_empty());
    let finished = vec![
        todo("write the migration", TodoStatus::Completed),
        todo("backfill old rows", TodoStatus::Completed),
        todo("update the docs", TodoStatus::Cancelled),
    ];
    let questions = asked(
        &mut engine,
        &turn_end_with(4, "All done; tests pass.", finished),
    );
    let delivered = answer(&mut engine, &questions, &[DONE]);
    assert_eq!(
        delivered,
        vec![(Delivery::Wait, "Goal achieved: ship it".to_string())]
    );
}

#[test]
fn a_multi_step_goal_without_todos_is_asked_to_write_them() {
    let mut engine = engine(shipped());

    settled(
        &mut engine,
        &command(1, RulebookCommand::Start, "migrate the API"),
    );
    let questions = asked(&mut engine, &turn_end(2, "Read the code."));
    let delivered = answer(&mut engine, &questions, &[TODOS]);

    assert_eq!(delivered.len(), 1, "{delivered:?}");
    assert_eq!(delivered[0].0, Delivery::Resume);
    assert!(
        delivered[0]
            .1
            .contains("Break the goal into todos with todowrite")
    );
}

#[test]
fn todo_gates_belong_to_turn_end_rules() {
    let rule = json!({
        "schema_version": 2,
        "id": "bad",
        "name": "Bad",
        "on": "tool_result",
        "when": { "tools": ["shell"], "todos": ["open"] },
        "steps": [{ "id": "q", "question": "q?", "yes_at_or_above": 0.7, "minimum_confidence": 0.4 }],
        "then": { "delivery": "steer", "text": "x" }
    });

    let error = Rule::from_json(rule.to_string().as_bytes()).unwrap_err();
    assert!(
        error.contains("when.todos applies only to turn_end"),
        "{error}"
    );
}

#[test]
fn commands_answer_without_asking() {
    let mut engine = engine(shipped());

    let missing = settled(&mut engine, &command(1, RulebookCommand::Start, "  "));
    assert!(missing[0].1.contains("needs arguments"), "{missing:?}");
    assert!(
        asked(&mut engine, &turn_end(2, "")).is_empty(),
        "not started"
    );

    let idle = settled(&mut engine, &command(3, RulebookCommand::Pause, ""));
    assert!(idle[0].1.contains("is not running"), "{idle:?}");

    settled(&mut engine, &command(4, RulebookCommand::Start, "x"));
    let cleared = settled(&mut engine, &command(5, RulebookCommand::Clear, ""));
    assert_eq!(cleared, vec![(Delivery::Wait, "Goal cleared.".to_string())]);
    assert!(asked(&mut engine, &turn_end(6, "")).is_empty());

    let unknown = signal(
        7,
        SignalKind::Rulebook {
            command: RulebookCommand::Start,
            rulebook: "nope".into(),
            args: String::new(),
            workspace: String::new(),
        },
    );
    assert!(
        settled(&mut engine, &unknown)[0]
            .1
            .contains("No rulebook named nope here. Rulebooks: /goal.")
    );
}

fn book(budget: u32) -> Value {
    json!({
        "schema_version": 1,
        "id": "goal",
        "name": "Goal",
        "description": "d",
        "budget": budget,
        "on_start": "start {args}",
        "on_budget": "budget spent on {args}",
        "rules": [{
            "schema_version": 2,
            "id": "goal-continue",
            "name": "continue",
            "on": "turn_end",
            "steps": [{ "id": "not-done", "question": "Is {args} unfinished?", "yes_at_or_above": 0.7, "minimum_confidence": 0.4 }],
            "then": { "delivery": "resume", "text": "keep going on {args}" }
        }]
    })
}

fn parse(value: &Value) -> Result<Rulebook, String> {
    Rulebook::from_json(&serde_json::to_vec(value).unwrap())
}

#[test]
fn a_spent_budget_asks_for_a_summary_and_stops() {
    let mut engine = engine(vec![parse(&book(1)).unwrap()]);

    settled(&mut engine, &command(1, RulebookCommand::Start, "x"));
    let questions = asked(&mut engine, &turn_end(2, ""));
    assert_eq!(
        answer(&mut engine, &questions, &[CONTINUE]),
        vec![(Delivery::Resume, "keep going on x".to_string())]
    );

    asked(&mut engine, &tool(3));
    let questions = asked(&mut engine, &turn_end(4, ""));
    assert_eq!(
        answer(&mut engine, &questions, &[CONTINUE]),
        vec![(Delivery::Resume, "budget spent on x".to_string())]
    );

    asked(&mut engine, &tool(5));
    assert!(
        asked(&mut engine, &turn_end(6, "")).is_empty(),
        "stopped by budget"
    );
}

#[test]
fn otherwise_continues_within_the_budget() {
    let mut value = book(1);
    value["otherwise"] = json!("not done: {args}");
    let mut engine = engine(vec![parse(&value).unwrap()]);

    settled(&mut engine, &command(1, RulebookCommand::Start, "x"));
    let questions = asked(&mut engine, &turn_end(2, ""));
    assert_eq!(
        answer(&mut engine, &questions, &[]),
        vec![(Delivery::Resume, "not done: x".to_string())]
    );

    // Nothing ran since: no check, and no `otherwise`.
    assert!(asked(&mut engine, &turn_end(3, "")).is_empty());

    asked(&mut engine, &tool(4));
    let questions = asked(&mut engine, &turn_end(5, ""));
    assert_eq!(
        answer(&mut engine, &questions, &[]),
        vec![(Delivery::Resume, "budget spent on x".to_string())]
    );
}

#[test]
fn rulebooks_are_validated() {
    let mut unprefixed = book(1);
    unprefixed["rules"][0]["id"] = json!("continue");
    assert!(
        parse(&unprefixed)
            .unwrap_err()
            .contains("must start with \"goal-\"")
    );

    let mut steer_end = book(1);
    steer_end["rules"][0]["on"] = json!("tool_result");
    steer_end["rules"][0]["when"] = json!({ "tools": ["shell"] });
    steer_end["rules"][0]["then"] = json!({ "delivery": "steer", "text": "t", "end": "complete" });
    assert!(
        parse(&steer_end)
            .unwrap_err()
            .contains("does not end a turn")
    );

    assert!(parse(&book(0)).unwrap_err().contains("budget"));

    let mut loose = book(1)["rules"][0].clone();
    loose["then"]["end"] = json!("complete");
    let error = Rule::from_json(&serde_json::to_vec(&loose).unwrap()).unwrap_err();
    assert!(error.contains("only inside a rulebook"), "{error}");
}
