//! Following the agent's own work: facts name a candidate, Jev confirms it,
//! and the integration creates a monitor unless one already watches it.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chauffeur_capability_monitors::{FollowWork, Monitor, Monitors, MonitorsConfig, Watch};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Delivery, Effect, Judging, Plan, Rule, Signal, SignalKind,
    Situation,
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

fn follow_work(monitors: Arc<Fake>) -> Judging<FollowWork> {
    Judging::new(FollowWork::new(monitors))
}

fn asked(capability: &mut Judging<FollowWork>, signal: &Signal) -> Vec<String> {
    match capability.plan(&Situation::default(), signal) {
        Plan::Ask(questions) => questions.into_iter().map(|question| question.id).collect(),
        _ => Vec::new(),
    }
}

#[test]
fn a_confirmed_pr_the_agent_opened_gets_a_monitor_and_the_agent_is_told() {
    let fake = Arc::new(Fake::default());
    let mut follow = follow_work(fake.clone());

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
    assert!(asked(&mut follow_work(watched), &pr_created()).is_empty());

    let fake = Arc::new(Fake::default());
    let mut follow = follow_work(fake.clone());

    asked(&mut follow, &pr_created());
    assert!(
        follow
            .decide(&pr_created(), Some(&[yes("follow/0", 0.2)]))
            .is_empty()
    );

    let mut retried = follow_work(fake.clone());

    asked(&mut retried, &pr_created());
    assert!(retried.decide(&pr_created(), None).is_empty());
    // A failed judgment may be asked again.
    assert_eq!(asked(&mut retried, &pr_created()), ["follow/0"]);
    assert!(fake.monitors.lock().unwrap().is_empty());
}

#[test]
fn reading_a_pr_or_an_unreachable_integration_asks_nothing() {
    let mut follow = follow_work(Arc::new(Fake::default()));
    let viewed = call(
        "shell",
        r#"{"command":"gh pr view 42"}"#,
        "https://github.com/acme/app/pull/42",
    );

    assert!(asked(&mut follow, &viewed).is_empty());
    assert!(
        asked(
            &mut follow_work(Arc::new(Fake {
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
    let mut follow = follow_work(Arc::new(Fake::default()));
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

fn yours(name: &str, json: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "chauffeur-monitors-{}-{name}.json",
        std::process::id()
    ));

    std::fs::write(&path, json).unwrap();
    path
}

#[test]
fn the_shipped_bar_follows_a_confident_yes_at_or_above_0_7() {
    let shipped = MonitorsConfig::load(Path::new("/nonexistent/monitors.json")).unwrap();

    assert_eq!(shipped, MonitorsConfig::default());
    assert_eq!(shipped.follow.yes(), Rule::yes(0.7, 0.4));
}

#[test]
fn your_bar_overrides_one_field_and_keeps_the_rest() {
    let path = yours("at", r#"{ "follow": { "at": 0.5 } }"#);

    assert_eq!(
        MonitorsConfig::load(&path).unwrap().follow.yes(),
        Rule::yes(0.5, 0.4)
    );
    std::fs::remove_file(path).unwrap();
}

#[test]
fn a_misspelled_field_or_an_out_of_range_bar_is_an_error() {
    let typo = yours("typo", r#"{ "folow": { "at": 0.5 } }"#);
    let range = yours("range", r#"{ "follow": { "at": -0.1 } }"#);

    assert!(MonitorsConfig::load(&typo).unwrap_err().contains("folow"));
    assert!(
        MonitorsConfig::load(&range)
            .unwrap_err()
            .contains("outside [0, 1]")
    );
    std::fs::remove_file(typo).unwrap();
    std::fs::remove_file(range).unwrap();
}

#[test]
fn a_looser_bar_follows_what_the_shipped_one_does_not() {
    let path = yours("loose", r#"{ "follow": { "at": 0.5 } }"#);
    let config = MonitorsConfig::load(&path).unwrap();
    let shipped = Arc::new(Fake::default());
    let loose = Arc::new(Fake::default());
    let mut follow = follow_work(shipped.clone());
    let mut loosened = Judging::new(FollowWork::new(loose.clone()).with_config(config));

    let sure = Answer {
        confidence: Some(0.9),
        ..yes("follow/0", 0.6)
    };

    asked(&mut follow, &pr_created());
    asked(&mut loosened, &pr_created());

    assert!(
        follow
            .decide(&pr_created(), Some(std::slice::from_ref(&sure)))
            .is_empty()
    );
    assert_eq!(
        loosened
            .decide(&pr_created(), Some(std::slice::from_ref(&sure)))
            .len(),
        1
    );
    assert!(shipped.monitors.lock().unwrap().is_empty());
    assert_eq!(loose.monitors.lock().unwrap().len(), 1);
    std::fs::remove_file(path).unwrap();
}
