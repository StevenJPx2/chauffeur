//! `chauffeur navigate` as a hook script runs it: the request on stdin, the
//! messages on stdout, the exit status. The daemon runs on a thread of this
//! test, on a private socket, with placeholders for every model.
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "the helpers fail the test on a broken fixture or output that is not JSON"
)]

use std::fs;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use chauffeur_daemon::Paths;
use serde_json::{Value, json};

const CHAUFFEUR: &str = env!("CARGO_BIN_EXE_chauffeur");

const PLACEHOLDERS: &str = r#"{ "models": { "stand-in": { "format": "placeholder",
                                                          "otherwise": { "yes": 0.1, "no": 0.9 } } },
                               "tiers": { "system-one": ["stand-in"] } }"#;

const TRIAGE: &str = r#"{
  "on": "sourcefed.event",
  "strategy": [{ "id": "screen", "judge": "system-one",
                 "ask": { "outcomes": ["yes", "no"], "evidence": ["event"],
                          "instructions": "Should the agent be shown this event now?" } }],
  "then": [{ "if": { "screen": "no" }, "withhold": "event" }]
}"#;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

/// A daemon on a thread, with the triage rule and `--models` placeholders.
fn daemon(name: &str) -> Paths {
    let root = std::env::temp_dir().join(format!(
        "chauffeur-cli-navigate-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("config/rules/sourcefed")).unwrap();
    fs::write(root.join("placeholders.json"), PLACEHOLDERS).unwrap();
    fs::write(root.join("config/rules/sourcefed/triage.json"), TRIAGE).unwrap();
    let paths = Paths {
        name: "chauffeur-test".into(),
        socket: root.join("run/daemon.sock"),
        config: root.join("config"),
        distribution: repo("distribution"),
        state: root.join("state"),
        models: Some(root.join("placeholders.json")),
    };
    let running = paths.clone();
    thread::spawn(move || chauffeur_daemon::run(&running));
    let started = Instant::now();
    while UnixStream::connect(&paths.socket).is_err() {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the daemon did not start"
        );
        thread::sleep(Duration::from_millis(20));
    }
    paths
}

/// Runs `chauffeur navigate` against `paths`' socket with `request` on stdin.
fn navigate(paths: &Paths, request: &str) -> Output {
    let mut child = Command::new(CHAUFFEUR)
        .arg("navigate")
        .env("CHAUFFEUR_SOCKET", &paths.socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn lines(output: &Output) -> Vec<Value> {
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn an_event_prints_its_events_then_the_intents_and_exits_0() {
    let paths = daemon("withheld");
    let request = r#"{ "protocol": 1, "deadline_ms": 3000, "on": "sourcefed.event",
                       "evidence": { "event": "thanks, looks good" } }"#;

    let output = navigate(&paths, request);

    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let messages = lines(&output);
    let events: Vec<&Value> = messages
        .iter()
        .map(|message| &message["params"]["event"])
        .collect();
    assert_eq!(
        events,
        [
            &json!("rule_matched"),
            &json!("round_ready"),
            &json!("step_settled"),
            &Value::Null
        ]
    );
    assert_eq!(
        messages[3]["result"]["intents"],
        json!([{ "rule": "sourcefed/triage", "withhold": "event" }])
    );
}

/// The fixtures every binding replays, each with its `fill` applied.
fn fixtures() -> Vec<Value> {
    let text = fs::read_to_string(repo("crates/protocol/fixtures/navigate.json")).unwrap();
    let fixtures: Value = serde_json::from_str(&text).unwrap();
    let mut cases = fixtures["cases"].as_array().unwrap().clone();
    for case in &mut cases {
        for fill in case["fill"].as_array().cloned().unwrap_or_default() {
            let (into, name) = (fill["in"].as_str().unwrap(), fill["name"].as_str().unwrap());
            let times = usize::try_from(fill["times"].as_u64().unwrap()).unwrap();
            case["params"][into][name] = json!(fill["text"].as_str().unwrap().repeat(times));
        }
    }
    cases
}

#[test]
fn every_fixture_is_answered_or_rejected_as_it_expects() {
    let paths = daemon("fixtures");

    for case in fixtures() {
        let name = case["name"].as_str().unwrap();
        let output = navigate(&paths, &case["params"].to_string());
        let reply = lines(&output).pop().unwrap();
        match case["reject"]["reason"].as_str() {
            None => {
                assert_eq!(output.status.code(), Some(0), "{name}: {reply}");
                let key = if case["expect"] == "on" {
                    "intents"
                } else {
                    "verdict"
                };
                assert!(reply["result"].get(key).is_some(), "{name}: {reply}");
            }
            Some(reason) => {
                let code = if reason == "unsupported_protocol" {
                    2
                } else {
                    1
                };
                assert_eq!(output.status.code(), Some(code), "{name}: {reply}");
                assert_eq!(reply["error"]["data"]["reason"], reason, "{name}");
                assert_eq!(
                    reply["error"]["data"].get("limit"),
                    case["reject"].get("limit"),
                    "{name}"
                );
            }
        }
    }
}

#[test]
fn with_no_daemon_it_exits_2_and_says_so() {
    let missing = std::env::temp_dir().join(format!(
        "chauffeur-cli-none-{}/daemon.sock",
        std::process::id()
    ));
    let paths = Paths {
        socket: missing,
        ..Paths::from_env("chauffeur-test", None)
    };

    let output = navigate(&paths, r#"{ "protocol": 1, "on": "x" }"#);

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("no daemon answers"));
}

#[test]
fn a_request_that_is_not_json_exits_1() {
    let paths = daemon("not-json");

    let output = navigate(&paths, "{ protocol: 1");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not JSON"));
}
