use std::sync::Arc;

use chauffeur_capability_event_gate::EventGate;
use chauffeur_capability_monitors::{Monitor, Monitors, Watch};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Effect, Judging, Plan, Signal, SignalKind, Situation,
};

fn event(summary: &str, monitor: &str) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::IntegrationEvent {
            source: "github".into(),
            kind: "comment".into(),
            summary: summary.into(),
            body: "Coverage 81.2% (+0.3%)".into(),
            actionable: true,
            monitor: monitor.into(),
        },
    }
}

/// Plan `signal`, then decide it with `answers`.
fn judged(
    gate: &mut Judging<EventGate>,
    signal: &Signal,
    answers: Option<&[Answer]>,
) -> Vec<Effect> {
    assert!(matches!(
        gate.plan(&Situation::default(), signal),
        Plan::Ask(_)
    ));

    gate.decide(signal, answers)
}

fn show(p: f32) -> Answer {
    Answer {
        id: "show".into(),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

fn instructions(gate: &mut Judging<EventGate>, signal: &Signal) -> String {
    let Plan::Ask(questions) = gate.plan(&Situation::default(), signal) else {
        panic!("expected a question")
    };

    questions[0].instructions.clone()
}

/// sourcefed's monitors for session `ses`: one on PR acme/app#42.
struct Fake;

impl Monitors for Fake {
    fn list(&self, agent_id: &str) -> Result<Vec<Monitor>, String> {
        Ok(if agent_id == "ses" {
            vec![Monitor {
                id: "mon_42".into(),
                watch: Some(Watch::GithubPr {
                    repo: "acme/app".into(),
                    number: 42,
                }),
                enabled: true,
            }]
        } else {
            Vec::new()
        })
    }

    fn create(&self, _: &str, _: &Watch) -> Result<Monitor, String> {
        Err("not used".into())
    }
}

#[test]
fn asks_whether_an_integration_event_needs_the_agent() {
    let asked = instructions(
        &mut Judging::new(EventGate::default()),
        &event("codecov[bot] commented on #42", ""),
    );

    assert!(asked.contains("codecov[bot] commented on #42\nCoverage 81.2%"));
    assert!(asked.contains("sourcefed marks it actionable"));
    assert_eq!(
        Judging::new(EventGate::default()).plan(
            &Situation::default(),
            &Signal {
                kind: SignalKind::TurnEnd {
                    workspace: String::new(),
                    user_request: String::new()
                },
                ..event("", "")
            }
        ),
        Plan::Skip
    );
}

#[test]
fn the_question_names_what_the_events_monitor_watches() {
    let mut gate = Judging::new(EventGate::with_monitors(Arc::new(Fake)));

    assert!(
        instructions(&mut gate, &event("CI failed", "mon_42"))
            .contains("It comes from this session's monitor on GitHub pull request acme/app#42.")
    );
    // An unknown monitor, or none, adds nothing.
    assert!(!instructions(&mut gate, &event("CI failed", "mon_other")).contains("It comes from"));
    assert!(!instructions(&mut gate, &event("CI failed", "")).contains("It comes from"));
}

#[test]
fn only_a_confident_no_withholds() {
    let signal = event("codecov[bot] commented on #42", "");
    let mut gate = Judging::new(EventGate::default());

    assert_eq!(
        judged(&mut gate, &signal, Some(&[show(0.1)])),
        vec![Effect::Gate {
            agent_id: "ses".into(),
            deliver: false
        }]
    );
    // Unsure, positive, and failed judgments all deliver.
    assert!(judged(&mut gate, &signal, Some(&[show(0.4)])).is_empty());
    assert!(judged(&mut gate, &signal, Some(&[show(0.9)])).is_empty());
    assert!(judged(&mut gate, &signal, None).is_empty());
}
