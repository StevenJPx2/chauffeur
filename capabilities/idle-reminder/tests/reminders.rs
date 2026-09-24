use chauffeur_capability_idle_reminder::{Gate, Rule};
use chauffeur_capability_idle_reminder::{IdleReminder, MAX_REMINDERS};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Delivery, Effect, Plan, Signal, SignalKind, Situation,
};

fn rule(id: &str, priority: u8) -> Rule {
    Rule::new(id, &id.to_uppercase(), "situation", &format!("do {id}")).priority(priority)
}

fn signal(at: u64, kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at,
        kind,
    }
}

fn tool(at: u64, name: &str) -> Signal {
    signal(
        at,
        SignalKind::ToolResult {
            tool: name.into(),
            ok: true,
            input: String::new(),
            error: String::new(),
        },
    )
}

fn asked(plan: &Plan) -> Vec<String> {
    match plan {
        Plan::Ask(questions) => questions
            .iter()
            .map(|question| question.id.clone())
            .collect(),
        other => panic!("expected questions, got {other:?}"),
    }
}

fn noul(id: &str, probability: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Noul(probability),
        confidence: None,
    }
}

fn reminded(effects: &[Effect]) -> Vec<&str> {
    effects
        .iter()
        .map(|effect| match effect {
            // A reminder wakes the agent, labelled with its rule.
            Effect::Context {
                delivery: Delivery::Resume,
                label,
                ..
            } => label.as_str(),
            other => panic!("expected reminders, got {other:?}"),
        })
        .collect()
}

#[test]
fn gates_use_the_tools_the_agent_ran() {
    let create_pr = rule("create-pr", 1).gate(
        Gate::default()
            .status(&["implementing"])
            .tools_not_called(&["github_open_pr"]),
    );
    let fix_ci = rule("fix-ci", 2).gate(Gate::default().status(&["in_review"]));
    let mut reminders = IdleReminder::new(vec![create_pr, fix_ci]);

    reminders.plan(&Situation::default(), &tool(1, "edit"));
    assert_eq!(
        asked(&reminders.plan(&Situation::default(), &signal(2, SignalKind::TurnEnd))),
        vec!["create-pr"]
    );

    reminders.plan(&Situation::default(), &tool(3, "github_open_pr"));
    assert_eq!(
        asked(&reminders.plan(&Situation::default(), &signal(4, SignalKind::TurnEnd))),
        vec!["fix-ci"]
    );
}

#[test]
fn confirmed_rules_remind_by_priority_up_to_the_limit() {
    let rules = vec![
        rule("low", 1),
        rule("high", 9),
        rule("mid", 5),
        rule("absent", 20),
    ];
    let mut reminders = IdleReminder::new(rules);
    let idle = signal(1, SignalKind::TurnEnd);
    let answers = [
        noul("low", 0.9),
        noul("high", 0.9),
        noul("mid", 0.9),
        noul("absent", 0.1),
    ];
    let effects = reminders.decide(&idle, Some(&answers));

    assert_eq!(reminded(&effects), vec!["high", "mid"]);
    assert_eq!(effects.len(), MAX_REMINDERS);
    assert!(
        matches!(&effects[0], Effect::Context { text: Some(text), .. } if text.contains("HIGH") && text.contains("do high"))
    );
}

#[test]
fn once_and_cooldown_suppress_repeat_reminders() {
    let mut reminders = IdleReminder::new(vec![
        rule("once", 1).once(),
        rule("cool", 1).cooldown_secs(60),
    ]);
    let answers = [noul("once", 0.9), noul("cool", 0.9)];

    assert_eq!(
        reminded(&reminders.decide(&signal(10, SignalKind::TurnEnd), Some(&answers))),
        vec!["once", "cool"]
    );
    assert_eq!(
        asked(&reminders.plan(&Situation::default(), &signal(20, SignalKind::TurnEnd))),
        Vec::<String>::new()
    );
    assert_eq!(
        asked(&reminders.plan(&Situation::default(), &signal(80, SignalKind::TurnEnd))),
        vec!["cool"]
    );
}

#[test]
fn a_failed_judgment_skips_the_nudge() {
    let mut reminders = IdleReminder::new(vec![rule("a", 1)]);

    assert!(
        reminders
            .decide(&signal(1, SignalKind::TurnEnd), None)
            .is_empty()
    );
}

#[test]
fn a_rule_needs_seventy_percent_by_default() {
    let mut reminders = IdleReminder::new(vec![rule("a", 1), rule("b", 1)]);
    let answers = [noul("a", 0.65), noul("b", 0.7)];

    assert_eq!(
        reminded(&reminders.decide(&signal(1, SignalKind::TurnEnd), Some(&answers))),
        vec!["b"]
    );
}

fn integration(at: u64, source: &str, kind: &str) -> Signal {
    signal(
        at,
        SignalKind::IntegrationEvent {
            source: source.into(),
            kind: kind.into(),
            summary: format!("{source} {kind}"),
            body: String::new(),
            actionable: true,
        },
    )
}

#[test]
fn integration_events_become_hooks_and_facts() {
    let create_pr = rule("create-pr", 1).gate(Gate::default().status(&["implementing"]));
    let transition =
        rule("transition", 2).gate(Gate::default().source(&["jira"]).hooks(&["github:merged"]));
    let mut reminders = IdleReminder::new(vec![create_pr, transition]);
    let idle = |at| signal(at, SignalKind::TurnEnd);

    reminders.plan(&Situation::default(), &tool(1, "edit"));
    assert_eq!(
        asked(&reminders.plan(&Situation::default(), &idle(2))),
        vec!["create-pr"]
    );

    // A Jira monitor on the ticket, then the PR (opened through the shell) merges.
    reminders.plan(&Situation::default(), &integration(3, "jira", "changelog"));
    reminders.plan(&Situation::default(), &integration(4, "github", "merged"));
    assert_eq!(
        asked(&reminders.plan(&Situation::default(), &idle(5))),
        vec!["transition"]
    );
}

#[test]
fn memory_survives_a_save_and_load() {
    let rules = || {
        vec![
            rule("create-pr", 1).gate(
                Gate::default()
                    .status(&["implementing"])
                    .tools_called(&["edit"]),
            ),
        ]
    };
    let mut before = IdleReminder::new(rules());

    before.plan(&Situation::default(), &tool(1, "edit"));

    let mut after = IdleReminder::new(rules());
    after.load(before.save().expect("idle reminders keep memory"));

    assert_eq!(
        asked(&after.plan(&Situation::default(), &signal(2, SignalKind::TurnEnd))),
        vec!["create-pr"]
    );
    // State that no longer fits is ignored.
    after.load(serde_json::json!({"unexpected": true}));
}
