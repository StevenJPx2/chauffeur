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
use chauffeur_runtime::{Context, Log, Ran, Redactor, run};

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

fn placeholder(answers: &str) -> Arc<Context> {
    context(
        "placeholder",
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

    let Ran::Settled { run: ran, then } = run(
        &placeholder(&answers),
        &commit_rule(),
        &commit(leaked),
        Duration::from_secs(2),
    ) else {
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
    let Ran::Settled { run: ran, then } = run(
        &placeholder(r#"{ "answers": [] }"#),
        &commit_rule(),
        &commit("fixed stuff"),
        Duration::from_secs(2),
    ) else {
        panic!("the rule did not settle")
    };

    assert_eq!(
        ran.verdict.get("nonconventional"),
        Some(&Outcome::Failed(Failure::Unavailable))
    );
    assert!(then.is_none());
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

#[test]
fn the_log_keeps_the_most_recent_entries_and_counts_the_rest() {
    let mut log = Log::new(2);
    for entry in 1..=5 {
        log.push(entry);
    }

    assert_eq!(log.iter().copied().collect::<Vec<_>>(), [4, 5]);
    assert_eq!(log.dropped(), 3);
}
