//! A judged capability through the engine: questions are namespaced and
//! merged, a chain runs across rounds, and a verdict is always acted on.

use chauffeur_core::judge::strategy::{self, Chained};
use chauffeur_core::{
    Answer, AnswerValue, Delivery, Effect, Engine, Judge, Judged, Judging, Question, QuestionKind,
    Rule, Signal, SignalKind, Situation, Step,
};

const YES: Rule = Rule::yes(0.7, 0.4);

/// Follows up on a turn: `pr` needs two steps, `changelog` one.
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

fn turn_end() -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::TurnEnd {
            workspace: String::new(),
            user_request: String::new(),
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

fn labels(step: &Step) -> Vec<&str> {
    let Step::Done(effects) = step else {
        panic!("expected effects, got {step:?}")
    };

    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Context { label, .. } => Some(label.as_str()),
            _ => None,
        })
        .collect()
}

fn ids(step: &Step) -> Vec<&str> {
    let Step::Ask { questions, .. } = step else {
        panic!("expected questions, got {step:?}")
    };

    questions
        .iter()
        .map(|question| question.id.as_str())
        .collect()
}

#[test]
fn a_chain_runs_across_the_engines_rounds() {
    let mut engine = engine();
    let first = engine.begin(&turn_end()).unwrap();

    assert_eq!(
        ids(&first),
        ["follow-up/pr/intent", "follow-up/changelog/notable"]
    );

    let second = engine.finish(
        Ok(vec![yes("pr/intent", 0.9), yes("changelog/notable", 0.9)]),
        1,
    );

    assert_eq!(ids(&second), ["follow-up/pr/ready"]);
    assert_eq!(
        labels(&engine.finish(Ok(vec![yes("pr/ready", 0.9)]), 1)),
        ["pr", "changelog"]
    );
}

#[test]
fn a_failed_round_settles_and_keeps_what_earlier_rounds_admitted() {
    let mut engine = engine();

    engine.begin(&turn_end()).unwrap();
    engine.finish(
        Ok(vec![yes("pr/intent", 0.9), yes("changelog/notable", 0.9)]),
        1,
    );

    assert_eq!(labels(&engine.finish(Err("down".into()), 1)), ["changelog"]);

    let mut failed = self::engine();

    failed.begin(&turn_end()).unwrap();
    assert!(labels(&failed.finish(Err("down".into()), 1)).is_empty());
}

#[test]
fn a_signal_the_capability_does_not_judge_is_skipped() {
    let message = Signal {
        kind: SignalKind::AgentRequest {
            need: "a browser".into(),
            user_request: String::new(),
            tools: Vec::new(),
            code_mode: Vec::new(),
        },
        ..turn_end()
    };

    assert_eq!(engine().begin(&message).unwrap(), Step::Done(Vec::new()));
}
