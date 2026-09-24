use chauffeur_capability_event_gate::EventGate;
use chauffeur_core::{
    Answer, AnswerValue, Capability, Effect, Plan, Signal, SignalKind, Situation,
};

fn event(summary: &str) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::IntegrationEvent {
            source: "github".into(),
            kind: "comment".into(),
            summary: summary.into(),
            body: "Coverage 81.2% (+0.3%)".into(),
            actionable: true,
        },
    }
}

fn show(p: f32) -> Answer {
    Answer {
        id: "show".into(),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

#[test]
fn asks_whether_an_integration_event_needs_the_agent() {
    let Plan::Ask(questions) = EventGate.plan(
        &Situation::default(),
        &event("codecov[bot] commented on #42"),
    ) else {
        panic!("expected a question")
    };

    assert!(
        questions[0]
            .instructions
            .contains("codecov[bot] commented on #42\nCoverage 81.2%")
    );
    assert!(
        questions[0]
            .instructions
            .contains("sourcefed marks it actionable")
    );
    assert_eq!(
        EventGate.plan(
            &Situation::default(),
            &Signal {
                kind: SignalKind::TurnEnd,
                ..event("")
            }
        ),
        Plan::Skip
    );
}

#[test]
fn only_a_confident_no_withholds() {
    let signal = event("codecov[bot] commented on #42");

    assert_eq!(
        EventGate.decide(&signal, Some(&[show(0.1)])),
        vec![Effect::Gate {
            agent_id: "ses".into(),
            deliver: false
        }]
    );
    // Unsure, positive, and failed judgments all deliver.
    assert!(EventGate.decide(&signal, Some(&[show(0.4)])).is_empty());
    assert!(EventGate.decide(&signal, Some(&[show(0.9)])).is_empty());
    assert!(EventGate.decide(&signal, None).is_empty());
}
