//! Several signals in flight at once: each run owns its judgment, rounds of
//! different runs interleave, and a result for a run that is gone changes
//! nothing.

use chauffeur_core::engine::{MAX_AGENTS, MAX_RUNS};
use chauffeur_core::judge::strategy::{self, Chained};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Delivery, Effect, Engine, Judge, Judged, Judging, Question,
    QuestionKind, Round, Rule, RunId, Signal, SignalKind, Situation, Step,
};

const YES: Rule = Rule::yes(0.7, 0.4);

/// At a turn end, `pr` needs two steps and `changelog` one.
struct FollowUp;

fn step(id: &str) -> (Question, Rule) {
    (
        Question {
            id: id.into(),
            instructions: format!("does {id} hold?"),
            kind: QuestionKind::Noul,
        },
        YES,
    )
}

impl Judged for FollowUp {
    type Verdict = Vec<String>;

    fn id(&self) -> &str {
        "follow-up"
    }

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<Vec<String>>> {
        matches!(signal.kind, SignalKind::TurnEnd { .. }).then(|| {
            strategy::chain([
                Chained {
                    key: "pr".to_string(),
                    steps: vec![step("pr/intent"), step("pr/ready")],
                },
                Chained {
                    key: "changelog".to_string(),
                    steps: vec![step("changelog/notable")],
                },
            ])
        })
    }

    fn act(&mut self, signal: &Signal, verdict: Vec<String>) -> Vec<Effect> {
        verdict
            .into_iter()
            .map(|label| Effect::Context {
                agent_id: signal.agent_id.clone(),
                delivery: Delivery::Resume,
                label,
                skills: Vec::new(),
                text: None,
            })
            .collect()
    }
}

fn engine() -> Engine {
    Engine::hosted(vec![Box::new(Judging::new(FollowUp))]).unwrap()
}

fn turn_end(agent: &str, summary: &str) -> Signal {
    Signal {
        agent_id: agent.into(),
        at: 1,
        kind: SignalKind::TurnEnd {
            workspace: String::new(),
            subagent: false,
            user_request: String::new(),
            summary: summary.into(),
            todos: Vec::new(),
        },
    }
}

/// A signal no capability judges, recorded and settled at once.
fn tool_call(agent: &str, at: u64) -> Signal {
    Signal {
        agent_id: agent.into(),
        at,
        kind: SignalKind::ToolResult {
            tool: "read".into(),
            ok: true,
            workspace: String::new(),
            subagent: false,
            input: String::new(),
            error: String::new(),
            user_request: String::new(),
            evidence: String::new(),
            candidates: Vec::new(),
        },
    }
}

fn yes(id: &str, p: f32) -> Answer {
    Answer {
        id: format!("follow-up/{id}"),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

fn asked(step: &Step) -> Vec<&str> {
    let Step::Ask { questions, .. } = step else {
        panic!("expected a round, got {step:?}")
    };

    questions
        .iter()
        .map(|question| question.id.as_str())
        .collect()
}

/// Each effect's agent and label.
fn effects(step: Option<Step>) -> Vec<(String, String)> {
    let Some(Step::Done(effects)) = step else {
        panic!("expected effects, got {step:?}")
    };

    effects
        .into_iter()
        .filter_map(|effect| match effect {
            Effect::Context {
                agent_id, label, ..
            } => Some((agent_id, label)),
            _ => None,
        })
        .collect()
}

fn pair(agent: &str, label: &str) -> (String, String) {
    (agent.into(), label.into())
}

#[test]
fn rounds_of_two_agents_interleave_and_each_settles_with_its_own_answers() {
    let mut engine = engine();
    let (a, first_a) = engine.start(&turn_end("a", "opened a PR")).unwrap();
    let (b, first_b) = engine.start(&turn_end("b", "fixed a typo")).unwrap();

    assert_ne!(a, b);
    assert_eq!(engine.active(), 2);
    assert_eq!(asked(&first_a), asked(&first_b));

    // b's first round comes back first, and only the changelog holds.
    let b_done = engine.resume(
        b,
        Ok(vec![yes("pr/intent", 0.1), yes("changelog/notable", 0.9)]),
        Vec::new(),
        1,
    );
    assert_eq!(effects(b_done), [pair("b", "changelog")]);

    // a needs a second round, then settles with both.
    let second_a = engine
        .resume(
            a,
            Ok(vec![yes("pr/intent", 0.9), yes("changelog/notable", 0.9)]),
            Vec::new(),
            1,
        )
        .unwrap();
    assert_eq!(asked(&second_a), ["follow-up/pr/ready"]);
    assert_eq!(engine.busy("a"), Some(a));
    assert_eq!(engine.busy("b"), None);

    let a_done = engine.resume(a, Ok(vec![yes("pr/ready", 0.9)]), Vec::new(), 1);
    assert_eq!(effects(a_done), [pair("a", "pr"), pair("a", "changelog")]);
    assert_eq!(engine.active(), 0);
}

#[test]
fn an_agent_has_one_run_in_flight_and_another_agent_is_not_held_up() {
    let mut engine = engine();
    let (a, _) = engine.start(&turn_end("a", "one")).unwrap();
    let error = engine.start(&turn_end("a", "two")).unwrap_err();

    assert!(error.contains("still judging"), "{error}");
    assert!(engine.start(&turn_end("b", "one")).is_ok());

    engine.resume(a, Err("down".into()), Vec::new(), 1).unwrap();
    assert!(engine.start(&turn_end("a", "two")).is_ok());
}

#[test]
fn a_result_for_a_run_that_is_gone_changes_nothing() {
    let mut engine = engine();
    let (a, _) = engine.start(&turn_end("a", "opened a PR")).unwrap();
    let answers = || Ok(vec![yes("pr/intent", 0.1), yes("changelog/notable", 0.9)]);

    assert!(
        engine
            .resume(RunId(999), answers(), Vec::new(), 1)
            .is_none()
    );
    assert!(engine.resume(a, answers(), Vec::new(), 1).is_some());
    // The same result again, as a duplicate or a late arrival.
    assert!(engine.resume(a, answers(), Vec::new(), 1).is_none());
}

#[test]
fn a_cancelled_run_never_acts_and_returns_what_it_had_asked() {
    let mut engine = engine();
    let (a, _) = engine.start(&turn_end("a", "opened a PR")).unwrap();
    let (b, _) = engine.start(&turn_end("b", "opened a PR")).unwrap();
    let trace = engine.cancel(a).unwrap();

    assert_eq!(trace.questions.len(), 2);
    assert_eq!(trace.error.as_deref(), Some("cancelled"));
    assert!(engine.cancel(a).is_none());
    assert!(
        engine
            .resume(a, Ok(vec![yes("pr/intent", 0.9)]), Vec::new(), 1)
            .is_none()
    );
    assert_eq!(engine.busy("a"), None);

    // The other run is untouched.
    let b_done = engine.resume(
        b,
        Ok(vec![yes("pr/intent", 0.1), yes("changelog/notable", 0.9)]),
        Vec::new(),
        1,
    );
    assert_eq!(effects(b_done), [pair("b", "changelog")]);
}

#[test]
fn the_trace_is_the_settled_runs_own() {
    let mut engine = engine();
    let (a, _) = engine.start(&turn_end("a", "opened a PR")).unwrap();
    let (b, _) = engine.start(&turn_end("b", "opened a PR")).unwrap();

    engine
        .resume(
            b,
            Ok(vec![yes("pr/intent", 0.9), yes("changelog/notable", 0.1)]),
            Vec::new(),
            7,
        )
        .unwrap();
    engine
        .resume(
            a,
            Ok(vec![yes("pr/intent", 0.1), yes("changelog/notable", 0.1)]),
            Vec::new(),
            3,
        )
        .unwrap();

    // a settled last: its trace is one round, not b's two.
    assert_eq!(engine.trace().elapsed_ms, 3);
    assert_eq!(engine.trace().answers.len(), 2);
    assert!(
        engine
            .trace()
            .questions
            .iter()
            .all(|question| question.round == 1)
    );
}

#[test]
fn runs_are_bounded() {
    let mut engine = engine();

    for agent in 0..MAX_RUNS {
        engine
            .start(&turn_end(&format!("agent-{agent}"), "x"))
            .unwrap();
    }

    let error = engine.start(&turn_end("one-more", "x")).unwrap_err();
    assert!(error.contains("in flight"), "{error}");
    // The bound is checked before planning, so even a signal that would
    // settle at once waits for a run to finish.
    assert!(engine.start(&tool_call("one-more", 2)).is_err());
}

#[test]
fn a_later_round_reads_the_activity_its_signal_saw_even_once_evicted() {
    let mut engine = engine();
    let (a, _) = engine.start(&turn_end("a", "opened PR #42")).unwrap();

    // Enough other agents to evict a's activity from the engine's memory.
    for agent in 0..MAX_AGENTS {
        engine
            .ingest(&tool_call(&format!("other-{agent}"), 10))
            .unwrap();
    }

    let second = engine
        .resume(
            a,
            Ok(vec![yes("pr/intent", 0.9), yes("changelog/notable", 0.1)]),
            Vec::new(),
            1,
        )
        .unwrap();
    let Step::Ask { state, .. } = second else {
        panic!("expected a second round")
    };

    assert!(state.contains("opened PR #42"), "{state}");
}

#[test]
fn begin_abandons_the_run_it_started_before() {
    let mut engine = engine();

    engine.begin(&turn_end("a", "first")).unwrap();
    engine.begin(&turn_end("b", "second")).unwrap();
    assert_eq!(engine.busy("a"), None);
    assert!(engine.busy("b").is_some());

    let done = engine.finish(
        Ok(vec![yes("pr/intent", 0.1), yes("changelog/notable", 0.9)]),
        1,
    );
    assert_eq!(effects(Some(done)), [pair("b", "changelog")]);
    assert_eq!(engine.finish(Err("late".into()), 1), Step::Done(Vec::new()));
}

#[test]
fn a_judged_capability_keeps_each_runs_judge_apart_and_forgets_cancelled_ones() {
    let mut follow = Judging::new(FollowUp);
    let (one, two) = (RunId(1), RunId(2));
    let situation = Situation::default();
    let answers = |p| {
        vec![
            Answer {
                id: "pr/intent".into(),
                value: AnswerValue::Noul(p),
                confidence: None,
            },
            Answer {
                id: "changelog/notable".into(),
                value: AnswerValue::Noul(0.1),
                confidence: None,
            },
        ]
    };

    follow.start(one, &situation, &turn_end("a", "x"));
    follow.start(two, &situation, &turn_end("b", "y"));
    assert_eq!(follow.in_flight(), 2);

    // Run one asks again; run two settles with nothing.
    let low = answers(0.1);
    let high = answers(0.9);
    assert!(matches!(
        follow.resume(two, &turn_end("b", "y"), &Round::answers(Some(&low)), 1),
        chauffeur_core::PipeStep::Done(effects) if effects.is_empty()
    ));
    assert!(matches!(
        follow.resume(one, &turn_end("a", "x"), &Round::answers(Some(&high)), 1),
        chauffeur_core::PipeStep::Next(_)
    ));
    assert_eq!(follow.in_flight(), 1);

    follow.cancel(one);
    assert_eq!(follow.in_flight(), 0);
}
