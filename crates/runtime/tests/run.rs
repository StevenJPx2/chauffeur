#![expect(
    clippy::unwrap_used,
    reason = "the helpers fail the test on a broken fixture"
)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use chauffeur_core::{Failure, Outcome};
use chauffeur_judges::Models;
use chauffeur_protocol::{Event, Rule};
use chauffeur_protocol::{Navigate, Target};
use chauffeur_runtime::{Cancel, Context, Ran, Redactor, run, run_strategy};

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn commit_rule() -> Arc<Rule> {
    let text = fs::read_to_string(repo("distribution/rules/conventional-commits.json")).unwrap();
    Arc::new(Rule::parse("conventional-commits", &text).unwrap())
}

fn commit(message: &str) -> Event {
    let event = serde_json::json!({
        "event": "tool.after", "session": "s1", "tool": "shell",
        "input": { "command": format!("git commit -m '{message}'") }
    });
    Event::parse(&event.to_string()).unwrap()
}

/// A context whose models come from `models` written to a fresh folder, with
/// the key `test-key` for every model.
fn context(name: &str, models: &str, files: &[(&str, &str)]) -> Arc<Context> {
    let folder =
        std::env::temp_dir().join(format!("chauffeur-runtime-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).unwrap();
    for (file, text) in files {
        fs::write(folder.join(file), text).unwrap();
    }
    fs::write(folder.join("models.json"), models).unwrap();
    Arc::new(Context {
        models: Models::load(&[folder.join("models.json")]).unwrap(),
        redactor: Redactor::load(&[repo("distribution/safety/redaction.json")]).unwrap(),
        keys: Box::new(|_| Some("test-key".to_owned())),
    })
}

/// `name` keeps each test's folder apart: `cargo test` runs them as threads
/// of one process.
fn placeholder(name: &str, answers: &str) -> Arc<Context> {
    context(
        name,
        r#"{ "models": { "stand-in": { "format": "placeholder", "answers": "answers.json" } },
             "tiers": { "system-one": ["stand-in"] } }"#,
        &[("answers.json", answers)],
    )
}

#[test]
fn a_judge_sees_only_redacted_evidence() {
    let leaked = "fixed AKIAABCDEFGHIJKLMNOP";
    let redacted = "tool: shell\ncommand: git commit -m 'fixed [REDACTED]'\n";
    let answers = format!(
        r#"{{ "answers": [{{ "evidence": {{ "tool_call": {redacted:?} }}, "p": {{ "yes": 0.94, "no": 0.06 }} }}] }}"#
    );

    let rule = commit_rule();
    let ran = run(
        &placeholder("redacted", &answers),
        &rule,
        &commit(leaked),
        Duration::from_secs(2),
    );
    let then = ran.then(&rule).cloned();
    let Ran::Settled(ran) = ran else {
        panic!("the rule did not settle")
    };

    let Some(Outcome::Value(answer)) = ran.verdict.get("nonconventional") else {
        panic!("no answer")
    };
    assert!(answer.by().placeholder);
    assert_eq!(
        then.map(|then| then.name().to_owned()).as_deref(),
        Some("message.append")
    );
}

#[test]
fn with_no_model_answering_the_step_fails_and_nothing_is_returned() {
    let rule = commit_rule();
    let ran = run(
        &placeholder("unanswered", r#"{ "answers": [] }"#),
        &rule,
        &commit("fixed stuff"),
        Duration::from_secs(2),
    );
    let then = ran.then(&rule);
    let Ran::Settled(ran) = &ran else {
        panic!("the rule did not settle")
    };

    assert_eq!(
        ran.verdict.get("nonconventional"),
        Some(&Outcome::Failed(Failure::Unavailable))
    );
    assert!(then.is_none());
}

#[test]
fn a_run_whose_worker_stops_is_refused_at_once_not_late() {
    let models = r#"{ "models": { "jev": { "format": "systemone", "endpoint": "http://127.0.0.1:9", "model": "jev-latest",
                                          "key": { "env": "TYPESAFE_API_KEY" } } },
                      "tiers": { "system-one": ["jev"] } }"#;
    let context = Arc::into_inner(context("stopped", models, &[])).unwrap();
    let context = Arc::new(Context {
        keys: Box::new(|_| panic!("the worker stops while it builds the judges")),
        ..context
    });

    let started = Instant::now();
    let ran = run(
        &context,
        &commit_rule(),
        &commit("fixed stuff"),
        Duration::from_secs(5),
    );

    assert!(
        matches!(&ran, Ran::Refused(reason) if reason.contains("worker")),
        "{ran:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_judge_slower_than_the_deadline_is_dropped() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut length = 0;
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            if line.trim().is_empty() {
                break;
            }
        }
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        thread::sleep(Duration::from_millis(800));
        let reply = r#"{"answers":{"q0":{"type":"noul","noul":0.9}}}"#;
        let _ = write!(
            reader.get_mut(),
            "HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n{reply}",
            reply.len()
        );
        String::from_utf8(body).unwrap()
    });
    let models = format!(
        r#"{{ "models": {{ "jev": {{ "format": "systemone", "endpoint": "{endpoint}", "model": "jev-latest",
                                  "key": {{ "env": "TYPESAFE_API_KEY" }}, "timeout_ms": 2000 }} }},
             "tiers": {{ "system-one": ["jev"] }} }}"#
    );

    let started = Instant::now();
    let ran = run(
        &context("deadline", &models, &[]),
        &commit_rule(),
        &commit("fixed AKIAABCDEFGHIJKLMNOP"),
        Duration::from_millis(200),
    );

    assert!(matches!(ran, Ran::Deadline), "{ran:?}");
    assert!(started.elapsed() < Duration::from_millis(600));
    let sent = server.join().unwrap();
    assert!(
        sent.contains("[REDACTED]") && !sent.contains("AKIA"),
        "{sent}"
    );
}

/// A stand-in for Jev on loopback that answers once, after `delay`.
fn slow_jev(delay: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else {
            return;
        };
        thread::sleep(delay);
        drop(stream);
    });
    format!(
        r#"{{ "models": {{ "jev": {{ "format": "systemone", "endpoint": "{endpoint}", "model": "jev-latest",
                                  "key": {{ "env": "TYPESAFE_API_KEY" }}, "timeout_ms": 5000 }} }},
             "tiers": {{ "system-one": ["jev"] }} }}"#
    )
}

fn inline(evidence: &str) -> (Arc<chauffeur_protocol::Strategy>, Vec<(String, String)>) {
    let navigate = Navigate::parse(&serde_json::json!({
        "protocol": 1,
        "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
        "strategy": [{ "id": "done", "judge": "quick",
                       "ask": { "outcomes": ["yes", "no"], "evidence": ["log"],
                                "instructions": "Is the migration finished?" } }],
        "evidence": { "log": evidence }
    }))
    .unwrap();
    let evidence = navigate.evidence().to_vec();
    let strategy = match navigate.into_target() {
        Target::Strategy(strategy) => Some(strategy),
        Target::On(_) => None,
    };
    let strategy = strategy.unwrap();
    (Arc::new(strategy), evidence)
}

fn in_ms(milliseconds: u64) -> Instant {
    Instant::now()
        .checked_add(Duration::from_millis(milliseconds))
        .unwrap()
}

#[test]
fn an_inline_strategy_settles_over_the_requests_evidence() {
    let answers = r#"{ "answers": [{ "evidence": { "log": "migrated 42 of 42 tables" },
                                     "p": { "yes": 0.9, "no": 0.1 } }] }"#;
    let (strategy, evidence) = inline("migrated 42 of 42 tables");

    let ran = run_strategy(
        &placeholder("inline", answers),
        &strategy,
        &evidence,
        in_ms(2_000),
        &Cancel::new(),
    );

    let Ran::Settled(ran) = ran else {
        panic!("the strategy did not settle: {ran:?}")
    };
    let Some(Outcome::Value(answer)) = ran.verdict.get("done") else {
        panic!("no answer")
    };
    assert_eq!(answer.outcome().to_string(), "yes");
}

#[test]
fn a_cancelled_run_returns_at_once_while_its_judge_is_still_asked() {
    let (strategy, evidence) = inline("migrated 41 of 42 tables");
    let context = context("cancelled", &slow_jev(Duration::from_secs(2)), &[]);
    let cancel = Cancel::new();
    let canceller = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        canceller.cancel();
    });

    let started = Instant::now();
    let ran = run_strategy(&context, &strategy, &evidence, in_ms(4_000), &cancel);

    assert!(matches!(ran, Ran::Cancelled), "{ran:?}");
    assert!(started.elapsed() < Duration::from_millis(1_000));
}
