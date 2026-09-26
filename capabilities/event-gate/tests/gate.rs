use std::path::{Path, PathBuf};
use std::sync::Arc;

use chauffeur_capability_event_gate::{EventGate, EventGateConfig};
use chauffeur_capability_monitors::{Monitor, Monitors, Watch};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Effect, Judging, Plan, Rule, Signal, SignalKind, Situation,
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

fn yours(name: &str, json: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "chauffeur-event-gate-{}-{name}.json",
        std::process::id()
    ));

    std::fs::write(&path, json).unwrap();
    path
}

#[test]
fn the_shipped_bar_withholds_a_confident_no_at_or_below_0_3() {
    let shipped = EventGateConfig::load(Path::new("/nonexistent/event-gate.json")).unwrap();

    assert_eq!(shipped, EventGateConfig::default());
    assert_eq!(shipped.withhold.no(), Rule::no(0.3, 0.4));
}

#[test]
fn your_bar_overrides_one_field_and_keeps_the_rest() {
    let path = yours("at", r#"{ "withhold": { "at": 0.2 } }"#);

    assert_eq!(
        EventGateConfig::load(&path).unwrap().withhold.no(),
        Rule::no(0.2, 0.4)
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_misspelled_field_or_an_out_of_range_bar_is_an_error() {
    let typo = yours("typo", r#"{ "withold": { "at": 0.2 } }"#);
    let range = yours("range", r#"{ "withhold": { "confidence": 1.5 } }"#);

    assert!(
        EventGateConfig::load(&typo)
            .unwrap_err()
            .contains("withold")
    );
    assert!(
        EventGateConfig::load(&range)
            .unwrap_err()
            .contains("outside [0, 1]")
    );
    std::fs::remove_file(typo).unwrap();
    std::fs::remove_file(range).unwrap();
}

#[test]
fn a_stricter_bar_delivers_what_the_shipped_one_withholds() {
    let path = yours("strict", r#"{ "withhold": { "at": 0.2 } }"#);
    let config = EventGateConfig::load(&path).unwrap();
    let signal = event("codecov[bot] commented on #42", "");

    assert!(
        !judged(
            &mut Judging::new(EventGate::default()),
            &signal,
            Some(&[show(0.25)])
        )
        .is_empty()
    );
    assert!(
        judged(
            &mut Judging::new(EventGate::default().with_config(config)),
            &signal,
            Some(&[show(0.25)])
        )
        .is_empty()
    );
    std::fs::remove_file(path).unwrap();
}
