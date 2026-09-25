//! Following the agent's own work: facts name a candidate, Jev confirms it,
//! and the integration creates a monitor unless one already watches it.

use std::sync::{Arc, Mutex};

use chauffeur_capability_monitors::{FollowWork, Monitor, Monitors, Watch};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Delivery, Effect, Plan, Signal, SignalKind, Situation,
};

/// An integration holding monitors in memory, or down.
#[derive(Default)]
struct Fake {
    monitors: Mutex<Vec<Monitor>>,
    down: bool,
}

impl Monitors for Fake {
    fn list(&self, _: &str) -> Result<Vec<Monitor>, String> {
        if self.down {
            return Err("connection refused".into());
        }

        Ok(self.monitors.lock().unwrap().clone())
    }

    fn create(&self, _: &str, watch: &Watch) -> Result<Monitor, String> {
        let monitor = Monitor {
            id: format!("mon_{}", watch.name()),
            watch: Some(watch.clone()),
            enabled: true,
        };

        self.monitors.lock().unwrap().push(monitor.clone());

        Ok(monitor)
    }
}

fn call(tool: &str, input: &str, output: &str) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::ToolResult {
            tool: tool.into(),
            ok: true,
            workspace: String::new(),
            input: input.into(),
            error: String::new(),
            user_request: String::new(),
            evidence: output.into(),
            candidates: Vec::new(),
        },
    }
}

fn pr_created() -> Signal {
    call(
        "shell",
        r#"{"command":"gh pr create --fill"}"#,
        "https://github.com/acme/app/pull/42",
    )
}

fn yes(id: &str, p: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

fn asked(capability: &mut FollowWork, signal: &Signal) -> Vec<String> {
    match capability.plan(&Situation::default(), signal) {
        Plan::Ask(questions) => questions.into_iter().map(|question| question.id).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn a_confirmed_pr_the_agent_opened_gets_a_monitor_and_the_agent_is_told() {
    let fake = Arc::new(Fake::default());
    let mut follow = FollowWork::new(fake.clone());

    assert_eq!(asked(&mut follow, &pr_created()), ["follow/0"]);

    let effects = follow.decide(&pr_created(), Some(&[yes("follow/0", 0.9)]));

    assert_eq!(
        fake.monitors
            .lock()
            .unwrap()
            .first()
            .and_then(|monitor| monitor.watch.clone()),
        Some(Watch::GithubPr {
            repo: "acme/app".into(),
            number: 42
        })
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::Context { delivery: Delivery::Steer, text: Some(text), .. }]
            if text.contains("GitHub pull request acme/app#42") && text.contains("Do not create another monitor")
    ));
    // Judged once: the same PR is not asked about again.
    assert!(asked(&mut follow, &pr_created()).is_empty());
}

#[test]
fn an_existing_monitor_a_no_or_a_failure_creates_nothing() {
    let watched = Arc::new(Fake::default());

    watched
        .create(
            "ses",
            &Watch::GithubPr {
                repo: "acme/app".into(),
                number: 42,
            },
        )
        .unwrap();
    assert!(asked(&mut FollowWork::new(watched), &pr_created()).is_empty());

    let fake = Arc::new(Fake::default());
    let mut follow = FollowWork::new(fake.clone());

    asked(&mut follow, &pr_created());
    assert!(
        follow
            .decide(&pr_created(), Some(&[yes("follow/0", 0.2)]))
            .is_empty()
    );

    let mut retried = FollowWork::new(fake.clone());

    asked(&mut retried, &pr_created());
    assert!(retried.decide(&pr_created(), None).is_empty());
    // A failed judgment may be asked again.
    assert_eq!(asked(&mut retried, &pr_created()), ["follow/0"]);
    assert!(fake.monitors.lock().unwrap().is_empty());
}

#[test]
fn reading_a_pr_or_an_unreachable_integration_asks_nothing() {
    let mut follow = FollowWork::new(Arc::new(Fake::default()));
    let viewed = call(
        "shell",
        r#"{"command":"gh pr view 42"}"#,
        "https://github.com/acme/app/pull/42",
    );

    assert!(asked(&mut follow, &viewed).is_empty());
    assert!(
        asked(
            &mut FollowWork::new(Arc::new(Fake {
                down: true,
                ..Fake::default()
            })),
            &pr_created()
        )
        .is_empty()
    );
}

#[test]
fn a_jira_issue_the_agent_works_on_is_a_candidate() {
    let mut follow = FollowWork::new(Arc::new(Fake::default()));
    let viewed = call(
        "shell",
        r#"{"command":"jira issue view ADEPT-45130"}"#,
        "Summary: fix nav spacing",
    );

    let Plan::Ask(questions) = follow.plan(&Situation::default(), &viewed) else {
        panic!("expected a question")
    };

    assert!(questions[0].instructions.contains("Jira issue ADEPT-45130"));
}
