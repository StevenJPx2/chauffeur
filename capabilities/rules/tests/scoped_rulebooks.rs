//! Scoped and project rulebooks, their inputs and skills, and books running
//! side by side in one session.

use std::path::PathBuf;

use chauffeur_capability_rules::{Input, Rulebook, Rules, rulebooks_for};
use chauffeur_core::{
    Answer, AnswerValue, Delivery, Effect, Engine, Judging, RulebookCommand, Signal, SignalKind,
    Step,
};
use serde_json::{Value, json};

fn book(id: &str, extra: Value) -> Value {
    let mut value = json!({
        "schema_version": 1,
        "id": id,
        "name": id.to_uppercase(),
        "description": "d",
        "budget": 5,
        "on_start": format!("{id} started on {{input}}"),
        "on_budget": "spent",
        "otherwise": format!("{id}: keep going on {{args}}"),
        "rules": [{
            "schema_version": 2,
            "id": format!("{id}-done"),
            "name": "done",
            "on": "turn_end",
            "steps": [{ "id": "done", "question": format!("Is {id} done for {{args}}?"), "yes_at_or_above": 0.7, "minimum_confidence": 0.4 }],
            "then": { "delivery": "wait", "text": format!("{id} done"), "end": "complete" }
        }]
    });

    if let (Some(value), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
        value.extend(extra.clone());
    }

    value
}

fn parse(value: &Value) -> Rulebook {
    Rulebook::from_json(&serde_json::to_vec(value).unwrap()).unwrap()
}

fn signal(at: u64, kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at,
        kind,
    }
}

fn start(at: u64, id: &str, args: &str, workspace: &str) -> Signal {
    signal(
        at,
        SignalKind::Rulebook {
            command: RulebookCommand::Start,
            rulebook: id.into(),
            args: args.into(),
            workspace: workspace.into(),
        },
    )
}

fn turn_end(at: u64) -> Signal {
    signal(
        at,
        SignalKind::TurnEnd {
            workspace: String::new(),
            subagent: false,
            user_request: String::new(),
            summary: String::new(),
            todos: Vec::new(),
        },
    )
}

/// Context effects as (delivery, text, skills).
fn contexts(effects: &[Effect]) -> Vec<(Delivery, String, Vec<String>)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Context {
                delivery,
                text,
                skills,
                ..
            } => Some((*delivery, text.clone().unwrap_or_default(), skills.clone())),
            _ => None,
        })
        .collect()
}

fn settled(engine: &mut Engine, signal: &Signal) -> Vec<(Delivery, String, Vec<String>)> {
    match engine.begin(signal).unwrap() {
        Step::Done(effects) => contexts(&effects),
        Step::Ask { questions, .. } => panic!("asked {questions:?}"),
    }
}

/// Ask `signal`'s questions and answer those in `yes` with 0.9, others 0.1.
fn judged(
    engine: &mut Engine,
    signal: &Signal,
    yes: &[&str],
) -> Vec<(Delivery, String, Vec<String>)> {
    let Step::Ask { questions, .. } = engine.begin(signal).unwrap() else {
        panic!("expected questions")
    };
    let answers = questions
        .iter()
        .map(|question| Answer {
            id: question.id.clone(),
            value: AnswerValue::Noul(if yes.contains(&question.id.as_str()) {
                0.9
            } else {
                0.1
            }),
            confidence: None,
        })
        .collect();

    match engine.finish(Ok(answers), 1) {
        Step::Done(effects) => contexts(&effects),
        Step::Ask { .. } => panic!("asked again"),
    }
}

#[test]
fn two_books_run_side_by_side_and_each_answers_for_itself() {
    let books = vec![
        parse(&book("goal", json!({}))),
        parse(&book("ticket", json!({}))),
    ];
    let mut engine = Engine::hosted(vec![Box::new(Judging::new(
        Rules::new(Vec::new()).with_rulebooks(books, true),
    ))])
    .unwrap();

    settled(&mut engine, &start(1, "ticket", "ADEPT-7", ""));
    settled(&mut engine, &start(2, "goal", "p95 under 120 ms", ""));

    // The goal is done; the ticket is not: each book answers once.
    let delivered = judged(&mut engine, &turn_end(3), &["rules/goal-done/done"]);
    let texts: Vec<&str> = delivered.iter().map(|(_, text, _)| text.as_str()).collect();

    assert_eq!(delivered.len(), 2, "{delivered:?}");
    assert!(texts.contains(&"goal done"), "{texts:?}");
    assert!(
        texts.contains(&"ticket: keep going on ADEPT-7"),
        "{texts:?}"
    );
}

#[test]
fn inputs_are_told_apart_and_bring_their_skills() {
    assert_eq!(Input::of("ADEPT-12345"), Input::Jira);
    assert_eq!(
        Input::of("https://acme.slack.com/archives/C0123/p1700000000000000"),
        Input::Slack
    );
    assert_eq!(Input::of("fix the ADEPT-12345 banner"), Input::Text);
    assert_eq!(Input::of("adept-1"), Input::Text);

    let skills = json!({ "skills": ["hpdp-overlay"], "input_skills": { "jira": "jira-cli", "slack": "slack-cli" } });
    let mut engine = Engine::hosted(vec![Box::new(Judging::new(
        Rules::new(Vec::new()).with_rulebooks(vec![parse(&book("ticket", skills))], true),
    ))])
    .unwrap();

    let jira = settled(&mut engine, &start(1, "ticket", "ADEPT-12345", ""));
    assert_eq!(jira[0].1, "ticket started on Jira ticket ADEPT-12345");
    assert_eq!(jira[0].2, ["hpdp-overlay", "jira-cli"]);

    let slack = settled(
        &mut engine,
        &start(
            2,
            "ticket",
            "https://acme.slack.com/archives/C0123/p1700000000000000",
            "",
        ),
    );
    assert!(
        slack[0]
            .1
            .ends_with("Slack thread https://acme.slack.com/archives/C0123/p1700000000000000")
    );
    assert_eq!(slack[0].2, ["hpdp-overlay", "slack-cli"]);

    let text = settled(&mut engine, &start(3, "ticket", "make the banner red", ""));
    assert_eq!(text[0].1, "ticket started on make the banner red");
    assert_eq!(text[0].2, ["hpdp-overlay"]);
}

fn scratch_repo(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("chauffeur-scoped-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join(".chauffeur/rulebooks")).unwrap();

    root.canonicalize().unwrap()
}

#[test]
fn scope_and_project_rulebooks_decide_what_a_workspace_offers() {
    let repo = scratch_repo("offer");
    let project = book("review", json!({}));
    std::fs::write(
        repo.join(".chauffeur/rulebooks/review.json"),
        serde_json::to_vec(&project).unwrap(),
    )
    .unwrap();

    let here = format!("{}/**", repo.display());
    let shipped = vec![
        parse(&book("goal", json!({}))),
        parse(&book("ticket", json!({ "scope": [here] }))),
        // A project book cannot shadow a shipped one.
        parse(&book("review", json!({ "scope": ["/nowhere/**"] }))),
    ];
    let ids = |workspace: &str| -> Vec<String> {
        rulebooks_for(shipped.clone(), workspace)
            .unwrap()
            .into_iter()
            .map(|book| book.id)
            .collect()
    };

    let inside = repo.join("pkg").display().to_string();
    std::fs::create_dir_all(&inside).unwrap();
    assert_eq!(ids(&inside), ["goal", "ticket"]);
    assert_eq!(ids("/tmp"), ["goal"]);

    std::fs::remove_file(repo.join(".chauffeur/rulebooks/review.json")).unwrap();
    std::fs::write(
        repo.join(".chauffeur/rulebooks/fix.json"),
        serde_json::to_vec(&book("fix", json!({}))).unwrap(),
    )
    .unwrap();
    assert_eq!(ids(&inside), ["goal", "ticket", "fix"]);

    let mut engine = Engine::hosted(vec![Box::new(Judging::new(
        Rules::new(Vec::new()).with_rulebooks(shipped.clone(), true),
    ))])
    .unwrap();

    assert!(
        settled(&mut engine, &start(1, "ticket", "x", "/tmp"))[0]
            .1
            .contains("No rulebook named ticket here")
    );
    assert_eq!(
        settled(&mut engine, &start(2, "fix", "x", &inside))[0].1,
        "fix started on x"
    );
    // A removed worktree still offers the shipped books in its scope.
    let removed = repo.join("gone").display().to_string();
    assert_eq!(
        settled(&mut engine, &start(3, "ticket", "x", &removed))[0].1,
        "ticket started on x"
    );

    let _ = std::fs::remove_dir_all(&repo);
}

#[test]
fn scopes_are_validated() {
    for scope in [
        json!(["relative/**"]),
        json!(["/a/*/b"]),
        json!(["/a/../b"]),
    ] {
        let error = Rulebook::from_json(
            &serde_json::to_vec(&book("x", json!({ "scope": scope }))).unwrap(),
        )
        .unwrap_err();
        assert!(error.contains("scope"), "{error}");
    }
}
