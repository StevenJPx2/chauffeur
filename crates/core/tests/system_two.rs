//! System Two through the engine: a judged capability screens with System
//! One and escalates to a System Two review, which runs on the engine's own
//! executor, sees redacted evidence, and is validated before it is read.

use std::sync::{Arc, Mutex};

use chauffeur_core::judge::strategy::{self, Escalation, Screened};
use chauffeur_core::{
    Answer, AnswerValue, Delivery, Effect, Engine, Evidence, Judge, Judged, Judging, Question,
    QuestionKind, Reasoned, Reasoning, ReasoningFailure, Rule, Schema, Signal, SignalKind,
    Situation, Step, SystemOne, SystemOneError, SystemTwo,
};
use serde_json::json;

const KEY: &str = "sk-live-4f9Qz7Lm2Xc8Vb6Nn1Rt5Yp3Wd0Ks9Gh";

/// Asks whether the turn finished the goal; reviews unsure and failed
/// screens with System Two, and resumes the agent only on a clear "continue".
struct Review {
    when: Escalation,
}

fn request() -> Reasoning {
    Reasoning {
        id: "review".into(),
        profile: "completion-review".into(),
        instructions: "Has the agent finished the goal?".into(),
        evidence: vec![Evidence {
            id: "summary".into(),
            text: format!("Tests pass. Deployed with key {KEY}."),
        }],
        activity: true,
        schema: Schema::Object(vec![
            (
                "verdict".into(),
                Schema::Enum(vec!["complete".into(), "continue".into()]),
            ),
            (
                "cites".into(),
                Schema::Array {
                    items: Box::new(Schema::EvidenceId),
                    max_items: 4,
                },
            ),
        ]),
    }
}

impl Judged for Review {
    /// Whether to resume the agent; `None` when nothing settled it.
    type Verdict = Option<bool>;

    fn id(&self) -> &str {
        "goal"
    }

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<Option<bool>>> {
        matches!(signal.kind, SignalKind::TurnEnd { .. }).then(|| {
            let question = Question {
                id: "done".into(),
                instructions: "Is the goal done?".into(),
                kind: QuestionKind::Noul,
            };

            strategy::escalate(
                strategy::screen(question, Rule::yes(0.7, 0.4), Rule::no(0.3, 0.4)),
                self.when,
                |screened| match screened {
                    Screened::Yes => Some(false),
                    Screened::No => Some(true),
                    Screened::Unsure | Screened::Failed => None,
                },
                |_| {
                    Judge::reason(request(), |outcome: Reasoned| {
                        outcome.ok().map(|verdict| verdict["verdict"] == "continue")
                    })
                },
            )
        })
    }

    fn act(&mut self, signal: &Signal, resume: Option<bool>) -> Vec<Effect> {
        let label = match resume {
            Some(true) => "continue",
            Some(false) => "complete",
            None => "unsettled",
        };

        vec![Effect::Context {
            agent_id: signal.agent_id.clone(),
            delivery: Delivery::Wait,
            label: label.into(),
            skills: Vec::new(),
            text: None,
        }]
    }
}

struct One(Result<f32, ()>);

impl SystemOne for One {
    fn name(&self) -> &str {
        "one"
    }

    fn ask(&mut self, _: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        let p = self.0.map_err(|()| SystemOneError("down".into()))?;

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

/// Replies with `reply` and keeps every request it was sent.
struct Two {
    reply: Reasoned,
    seen: Arc<Mutex<Vec<Reasoning>>>,
}

impl SystemTwo for Two {
    fn name(&self) -> &str {
        "two"
    }

    fn reason(&mut self, request: &Reasoning) -> Reasoned {
        self.seen.lock().unwrap().push(request.clone());
        self.reply.clone()
    }
}

fn engine(
    one: Result<f32, ()>,
    two: Option<Reasoned>,
    when: Escalation,
) -> (Engine, Arc<Mutex<Vec<Reasoning>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let engine = Engine::new(
        Box::new(One(one)),
        vec![Box::new(Judging::new(Review { when }))],
    )
    .unwrap();
    let engine = match two {
        Some(reply) => engine.with_system_two(Box::new(Two {
            reply,
            seen: Arc::clone(&seen),
        })),
        None => engine,
    };

    (engine, seen)
}

fn turn_end() -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::TurnEnd {
            workspace: String::new(),
            subagent: false,
            user_request: "ship the release".into(),
            summary: "Released.".into(),
            todos: Vec::new(),
        },
    }
}

fn label(effects: &[Effect]) -> &str {
    match effects {
        [Effect::Context { label, .. }] => label,
        other => panic!("expected one context effect, got {other:?}"),
    }
}

const BOTH: Escalation = Escalation {
    unsure: true,
    failed: true,
};

fn continue_verdict() -> Reasoned {
    Ok(json!({ "verdict": "continue", "cites": ["summary"] }))
}

#[test]
fn a_confident_screen_never_calls_system_two() {
    let (mut engine, seen) = engine(Ok(0.95), Some(continue_verdict()), BOTH);

    assert_eq!(label(&engine.ingest(&turn_end()).unwrap()), "complete");
    assert!(seen.lock().unwrap().is_empty());
    assert!(engine.trace().reasonings.is_empty());
}

#[test]
fn an_unsure_screen_is_reviewed_with_redacted_evidence_and_activity() {
    let (mut engine, seen) = engine(Ok(0.5), Some(continue_verdict()), BOTH);

    assert_eq!(label(&engine.ingest(&turn_end()).unwrap()), "continue");

    let seen = seen.lock().unwrap();
    let [sent] = seen.as_slice() else {
        panic!("expected one reasoning, got {}", seen.len())
    };
    let ids: Vec<&str> = sent
        .evidence
        .iter()
        .map(|piece| piece.id.as_str())
        .collect();

    assert_eq!(sent.id, "goal/review");
    assert_eq!(ids, ["activity", "summary"]);
    assert!(
        sent.evidence[0]
            .text
            .contains("Agent ended its turn: Released.")
    );
    assert!(
        !sent.evidence[1].text.contains(KEY),
        "{}",
        sent.evidence[1].text
    );

    let traced = &engine.trace().reasonings;
    assert_eq!(traced.len(), 1);
    assert_eq!((traced[0].round, traced[0].verdict.is_some()), (2, true));
}

#[test]
fn a_failed_screen_falls_back_to_system_two_when_the_policy_says_so() {
    let (mut engine, seen) = engine(Err(()), Some(continue_verdict()), BOTH);

    assert_eq!(label(&engine.ingest(&turn_end()).unwrap()), "continue");
    assert_eq!(seen.lock().unwrap().len(), 1);

    let only_unsure = Escalation {
        unsure: true,
        failed: false,
    };
    let (mut settled, seen) = self::engine(Err(()), Some(continue_verdict()), only_unsure);

    assert_eq!(label(&settled.ingest(&turn_end()).unwrap()), "unsettled");
    assert!(seen.lock().unwrap().is_empty());
}

#[test]
fn without_a_valid_verdict_the_capability_applies_its_failure_posture() {
    let invalid = Ok(json!({ "verdict": "continue", "cites": ["made-up"] }));

    for two in [None, Some(Err(ReasoningFailure::Refused)), Some(invalid)] {
        let (mut engine, _) = engine(Ok(0.5), two.clone(), BOTH);

        assert_eq!(
            label(&engine.ingest(&turn_end()).unwrap()),
            "unsettled",
            "{two:?}"
        );
        assert!(engine.trace().reasonings[0].failure.is_some());
    }
}

#[test]
fn a_hosted_engine_hands_its_host_the_reasoning_and_checks_what_comes_back() {
    let mut engine = Engine::hosted(vec![Box::new(Judging::new(Review { when: BOTH }))]).unwrap();
    let Step::Ask { questions, .. } = engine.begin(&turn_end()).unwrap() else {
        panic!("expected the screen")
    };
    let unsure = Answer {
        id: questions[0].id.clone(),
        value: AnswerValue::Noul(0.5),
        confidence: None,
    };
    let Step::Ask {
        questions,
        reasonings,
        ..
    } = engine.finish(Ok(vec![unsure]), 1)
    else {
        panic!("expected a reasoning round")
    };

    assert!(questions.is_empty());
    assert_eq!(reasonings[0].id, "goal/review");

    // A host's verdict that cites unknown evidence is not read.
    let Step::Done(effects) = engine.finish_round(
        Err("not asked".into()),
        vec![(
            "goal/review".into(),
            Ok(json!({ "verdict": "continue", "cites": ["nope"] })),
        )],
        3,
    ) else {
        panic!("expected done")
    };

    assert_eq!(label(&effects), "unsettled");
}
