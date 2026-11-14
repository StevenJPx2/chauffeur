//! `chauffeur.navigate` through the daemon's real socket: intents and
//! verdicts, the events streamed before them, rejections, several calls on
//! one connection, cancellation and admission.
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "the helpers fail the test on a broken fixture or a reply missing a field"
)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use chauffeur_daemon::Paths;
use serde_json::{Value, json};

const TRIAGE: &str = r#"{
  "on": "sourcefed.event",
  "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
  "strategy": [{ "id": "screen", "judge": "quick",
                 "ask": { "outcomes": ["yes", "no"], "evidence": ["event"],
                          "instructions": "Should the agent be shown this event now?" } }],
  "then": [{ "if": { "screen": "no" }, "withhold": "event" }]
}"#;

const ANSWERS: &str = r#"{ "answers": [
  { "evidence": { "event": "codecov[bot]: Coverage 81.2%" }, "p": { "yes": 0.1, "no": 0.9 } },
  { "evidence": { "event": "CI failed" }, "p": { "yes": 0.95, "no": 0.05 } },
  { "evidence": { "log": "migrated 42 of 42 tables" }, "p": { "yes": 0.9, "no": 0.1 } }
] }"#;

/// A stand-in for Jev on loopback that answers every question yes, each
/// request after `delay`, as `slow` in a models file whose `stand-in`
/// placeholder reads `answers.json`.
fn models(delay: Duration) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming().filter_map(Result::ok) {
            thread::spawn(move || {
                let mut reader = BufReader::new(stream);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let _ = reader.read_exact(&mut vec![0; length]);
                thread::sleep(delay);
                let body = r#"{"answers":{"q0":{"type":"noul","noul":0.97}}}"#;
                let _ = write!(
                    reader.get_mut(),
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    json!({
        "models": {
            "stand-in": { "format": "placeholder", "answers": "answers.json" },
            "slow": { "format": "systemone", "endpoint": endpoint, "model": "jev-latest",
                      "key": { "env": "HOME" }, "timeout_ms": 5000 }
        },
        "tiers": { "system-one": ["stand-in"] }
    })
    .to_string()
}

/// Fresh paths with the triage rule as an integration's, and `models`.
fn paths(name: &str, models: &str) -> Paths {
    let root =
        std::env::temp_dir().join(format!("chauffeur-navigate-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("config/rules/sourcefed")).unwrap();
    fs::write(root.join("config/models.json"), models).unwrap();
    fs::write(root.join("config/answers.json"), ANSWERS).unwrap();
    fs::write(root.join("config/rules/sourcefed/triage.json"), TRIAGE).unwrap();
    Paths {
        name: "chauffeur-test".into(),
        socket: root.join("run/daemon.sock"),
        config: root.join("config"),
        distribution: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../distribution"),
        state: root.join("state"),
        models: None,
    }
}

fn start(paths: &Paths) -> UnixStream {
    let running = paths.clone();
    thread::spawn(move || chauffeur_daemon::run(&running));
    let started = Instant::now();
    loop {
        if let Ok(stream) = UnixStream::connect(&paths.socket) {
            return stream;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the daemon did not start"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// A connection: requests written to it, messages read from it.
struct Caller {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
}

impl Caller {
    fn new(stream: UnixStream) -> Self {
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let reader = BufReader::new(stream.try_clone().unwrap());
        Self { stream, reader }
    }

    fn navigate(&mut self, id: u64, params: &Value) {
        let request =
            json!({ "jsonrpc": "2.0", "id": id, "method": "chauffeur.navigate", "params": params });
        writeln!(self.stream, "{request}").unwrap();
    }

    fn cancel(&mut self, id: u64) {
        let request =
            json!({ "jsonrpc": "2.0", "method": "$/cancelRequest", "params": { "id": id } });
        writeln!(self.stream, "{request}").unwrap();
    }

    fn message(&mut self) -> Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    /// Messages until the reply to `id`, which is last.
    fn until_reply(&mut self, id: u64) -> Vec<Value> {
        let mut messages = Vec::new();
        loop {
            let message = self.message();
            let done = message["id"] == id;
            messages.push(message);
            if done {
                return messages;
            }
        }
    }
}

fn event(text: &str) -> Value {
    json!({ "protocol": 1, "on": "sourcefed.event", "deadline_ms": 3000,
            "evidence": { "event": text, "monitor": "PR #42", "thread": "feat: navigate" } })
}

#[test]
fn noise_is_withheld_with_the_events_of_its_run_before_the_intents() {
    let paths = paths("withheld", &models(Duration::ZERO));
    let mut caller = Caller::new(start(&paths));

    caller.navigate(1, &event("codecov[bot]: Coverage 81.2%"));
    let messages = caller.until_reply(1);

    let events: Vec<&Value> = messages.iter().map(|message| &message["params"]).collect();
    assert_eq!(messages.len(), 4, "{messages:?}");
    assert_eq!(messages[0]["method"], "chauffeur.event");
    assert_eq!(
        *events[0],
        json!({ "id": 1, "event": "rule_matched", "rule": "sourcefed/triage" })
    );
    assert_eq!(events[1]["event"], "round_ready");
    assert_eq!(events[1]["steps"], json!(["screen"]));
    assert_eq!(events[2]["event"], "step_settled");
    assert_eq!(events[2]["answer"]["outcome"], "no");
    assert_eq!(events[2]["answer"]["placeholder"], true);
    assert_eq!(
        messages[3]["result"],
        json!({ "intents": [{ "rule": "sourcefed/triage", "withhold": "event" }] })
    );

    caller.navigate(2, &event("CI failed"));
    let delivered = caller.until_reply(2);
    assert_eq!(
        delivered.last().unwrap()["result"],
        json!({ "intents": [] })
    );
}

#[test]
fn an_event_no_rule_hooks_returns_no_intents_and_no_events() {
    let paths = paths("unhooked", &models(Duration::ZERO));
    let mut caller = Caller::new(start(&paths));

    caller.navigate(
        1,
        &json!({ "protocol": 1, "on": "neo.screen-check", "evidence": {} }),
    );

    assert_eq!(
        caller.until_reply(1),
        [json!({ "jsonrpc": "2.0", "id": 1, "result": { "intents": [] } })]
    );
}

#[test]
fn an_inline_strategy_returns_its_verdict() {
    let paths = paths("inline", &models(Duration::ZERO));
    let mut caller = Caller::new(start(&paths));

    caller.navigate(
        1,
        &json!({
            "protocol": 1,
            "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
            "strategy": [{ "id": "done", "judge": "quick",
                           "ask": { "outcomes": ["yes", "no"], "evidence": ["log"],
                                    "instructions": "Is the migration finished?" } }],
            "evidence": { "log": "migrated 42 of 42 tables" }
        }),
    );
    let messages = caller.until_reply(1);

    let verdict = &messages.last().unwrap()["result"]["verdict"];
    assert_eq!(verdict["done"]["answer"]["outcome"], "yes", "{messages:?}");
    assert_eq!(messages[0]["params"]["event"], "round_ready");
    assert!(messages[0]["params"].get("rule").is_none());
}

#[test]
fn a_request_over_a_limit_or_of_another_protocol_is_rejected_by_reason() {
    let paths = paths("rejected", &models(Duration::ZERO));
    let mut caller = Caller::new(start(&paths));

    caller.navigate(1, &json!({ "protocol": 2, "on": "x" }));
    caller.navigate(
        2,
        &json!({ "protocol": 1, "on": "x", "deadline_ms": 30_001 }),
    );
    // Each call runs on its own thread, so the replies may come in either order.
    let mut replies = [caller.message(), caller.message()];
    replies.sort_by_key(|reply| reply["id"].as_u64());
    let [first, second] = replies;

    assert_eq!(
        first["error"]["data"],
        json!({ "reason": "unsupported_protocol", "accepted": [1] })
    );
    assert_eq!(second["error"]["data"]["limit"], "deadline_ms");
    assert_eq!(second["error"]["code"], -32_602);
}

#[test]
fn an_inline_strategy_with_no_model_for_its_judge_is_unavailable() {
    let paths = paths("unavailable", &models(Duration::ZERO));
    let mut caller = Caller::new(start(&paths));

    caller.navigate(
        1,
        &json!({
            "protocol": 1,
            "strategy": [{ "id": "review", "judge": "system-two",
                           "ask": { "outcomes": ["yes", "no"], "instructions": "Is it safe?" } }]
        }),
    );

    assert_eq!(
        caller.message()["error"]["data"]["reason"],
        "judge_unavailable"
    );
}

fn slow_inline(deadline_ms: u64) -> Value {
    json!({
        "protocol": 1, "deadline_ms": deadline_ms,
        "judges": { "careful": { "tier": "system-one", "model": "slow" } },
        "strategy": [{ "id": "done", "judge": "careful",
                       "ask": { "outcomes": ["yes", "no"], "instructions": "Is it finished?" } }]
    })
}

#[test]
fn an_inline_strategy_past_its_deadline_is_rejected_as_deadline() {
    let paths = paths("deadline", &models(Duration::from_secs(2)));
    let mut caller = Caller::new(start(&paths));

    let started = Instant::now();
    caller.navigate(1, &slow_inline(200));

    assert_eq!(caller.message()["error"]["data"]["reason"], "deadline");
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn one_connection_carries_several_calls_and_a_fast_one_is_not_held_up() {
    let paths = paths("several", &models(Duration::from_millis(800)));
    let mut caller = Caller::new(start(&paths));

    caller.navigate(1, &slow_inline(3_000));
    caller.navigate(2, &event("CI failed"));

    let first = caller.message();
    assert_eq!(
        first["params"]["id"], 2,
        "the fast call's events come first: {first}"
    );
    let replies: Vec<Value> = caller
        .until_reply(1)
        .into_iter()
        .filter(|m| m.get("id").is_some())
        .collect();
    assert_eq!(replies[0]["id"], 2);
    assert_eq!(
        replies.last().unwrap()["result"]["verdict"]["done"]["answer"]["outcome"],
        "yes"
    );
}

#[test]
fn a_cancelled_request_is_answered_as_cancelled_at_once() {
    let paths = paths("cancel", &models(Duration::from_secs(3)));
    let mut caller = Caller::new(start(&paths));

    caller.navigate(1, &slow_inline(10_000));
    thread::sleep(Duration::from_millis(100));
    let started = Instant::now();
    caller.cancel(1);
    let reply = caller.message();

    assert_eq!(reply["error"]["code"], -32_800, "{reply}");
    assert_eq!(reply["error"]["data"]["reason"], "cancelled");
    assert!(started.elapsed() < Duration::from_secs(1));
}

/// The audit log's runs, oldest first.
fn runs(paths: &Paths) -> Vec<Value> {
    chauffeur_daemon::audit::recent(&paths.state, 256)
        .unwrap()
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn closing_the_connection_cancels_its_calls_and_frees_their_places() {
    let paths = paths("closed", &models(Duration::from_secs(3)));
    let mut caller = Caller::new(start(&paths));
    for id in 1..=8 {
        caller.navigate(id, &slow_inline(10_000));
    }
    thread::sleep(Duration::from_millis(200));

    drop(caller);

    let started = Instant::now();
    while runs(&paths)
        .iter()
        .filter(|run| run["cancelled"] == true)
        .count()
        < 8
    {
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            runs(&paths)
        );
        thread::sleep(Duration::from_millis(20));
    }
    let mut next = Caller::new(UnixStream::connect(&paths.socket).unwrap());
    next.navigate(9, &event("CI failed"));
    assert!(next.until_reply(9).last().unwrap().get("result").is_some());
}

#[test]
fn past_a_sources_share_a_navigation_is_busy_at_once() {
    let paths = paths("busy", &models(Duration::from_secs(1)));
    let mut caller = Caller::new(start(&paths));

    for id in 1..=10 {
        caller.navigate(id, &slow_inline(5_000));
    }
    let mut replies = Vec::new();
    while replies.len() < 10 {
        let message = caller.message();
        if message.get("method").is_none() {
            replies.push(message);
        }
    }

    let busy = replies
        .iter()
        .filter(|reply| reply["error"]["data"]["reason"] == "busy")
        .count();
    let answered = replies
        .iter()
        .filter(|reply| reply.get("result").is_some())
        .count();
    assert_eq!((busy, answered), (2, 8), "{replies:?}");
}

#[test]
fn two_matching_rules_both_return_and_a_late_one_drops_only_its_own() {
    let paths = paths("two-rules", &models(Duration::from_secs(2)));
    fs::write(
        paths.config.join("rules/sourcefed/also.json"),
        TRIAGE.replace("\"withhold\": \"event\"", "\"label\": \"noise\""),
    )
    .unwrap();
    fs::write(
        paths.config.join("rules/sourcefed/late.json"),
        TRIAGE
            .replace("\"confidence\": 0.7", "\"model\": \"slow\"")
            .replace("\"no\" }", "\"yes\" }"),
    )
    .unwrap();
    let mut caller = Caller::new(start(&paths));

    let started = Instant::now();
    let mut request = event("codecov[bot]: Coverage 81.2%");
    request["deadline_ms"] = json!(500);
    caller.navigate(1, &request);
    let messages = caller.until_reply(1);

    assert!(started.elapsed() < Duration::from_millis(1_500));
    assert_eq!(
        messages.last().unwrap()["result"]["intents"],
        json!([{ "rule": "sourcefed/also", "label": "noise" }, { "rule": "sourcefed/triage", "withhold": "event" }])
    );
    let matched = messages
        .iter()
        .filter(|m| m["params"]["event"] == "rule_matched")
        .count();
    assert_eq!(matched, 3);
    assert!(
        runs(&paths)
            .iter()
            .any(|run| run["rule"] == "sourcefed/late" && run["deadline"] == true)
    );
}
