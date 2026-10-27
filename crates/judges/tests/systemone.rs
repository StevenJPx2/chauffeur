//! The `systemone` format against a loopback HTTP server that replays the
//! shape captured from the live Jev API (jev-1.13.0).
#![expect(
    clippy::unwrap_used,
    reason = "the loopback server's helpers fail the test on a broken fixture"
)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chauffeur_core::Failure;
use chauffeur_judges::{
    Evidence, Format, KeySource, Outcomes, Question, SystemOne, SystemOneEntry,
};

const LIVE_RESPONSE: &str = r#"{"model":"jev-1.13.0","answers":{"q0":{"type":"noul","noul":0.97}},"usage":{"input_tokens":397,"output_tokens":73}}"#;

/// What the server received: the path, the authorization header and the body.
struct Received {
    path: String,
    authorization: String,
    body: serde_json::Value,
}

/// Serves one request with `status` and `body`, after `delay`.
fn serve(
    status: &'static str,
    body: &'static str,
    delay: Duration,
) -> (String, JoinHandle<Received>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line).unwrap();
        let (mut authorization, mut length) = (String::new(), 0);
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            if lower.starts_with("authorization:") {
                line.trim().clone_into(&mut authorization);
            }
        }
        let mut request = vec![0; length];
        reader.read_exact(&mut request).unwrap();
        thread::sleep(delay);
        let _ = write!(
            reader.get_mut(),
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        Received {
            path: request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned(),
            authorization,
            body: serde_json::from_slice(&request).unwrap(),
        }
    });
    (endpoint, server)
}

fn jev(endpoint: String, timeout_ms: u64) -> SystemOne {
    let entry = SystemOneEntry {
        endpoint,
        model: "jev-latest".into(),
        key: KeySource {
            env: "TYPESAFE_API_KEY".into(),
        },
        timeout_ms: Some(timeout_ms),
    };
    SystemOne::new(&entry, "test-key".into())
}

fn commit_question() -> Question {
    Question::noul(
        "Does this commit message break Conventional Commits?",
        ["commit"],
    )
    .unwrap()
}

fn evidence() -> Evidence {
    Evidence::new([("commit", "fixed stuff"), ("unused", "never sent")])
}

#[test]
fn sends_one_noul_request_and_reads_the_live_answer() {
    let (endpoint, server) = serve("200 OK", LIVE_RESPONSE, Duration::ZERO);
    let question = commit_question();

    let replies = jev(endpoint, 5_000).answer_all(&[&question], &evidence());
    let received = server.join().unwrap();

    assert_eq!(received.path, "/v1/systemone");
    assert_eq!(received.authorization, "authorization: Bearer test-key");
    assert_eq!(received.body["model"], "jev-latest");
    assert_eq!(received.body["questions"]["q0"]["type"], "noul");
    assert_eq!(
        received.body["questions"]["q0"]["instructions"],
        question.instructions()
    );
    assert_eq!(received.body["state"], "commit:\nfixed stuff\n\n");
    let p = replies.into_iter().next().unwrap().unwrap();
    assert_eq!(
        p.as_slice().iter().map(|p| p.get()).collect::<Vec<_>>(),
        [970, 30]
    );
}

#[test]
fn provider_errors_become_failures_that_say_nothing_else() {
    let cases = [
        ("429 Too Many Requests", "{}", Failure::RateLimited),
        (
            "529 Overloaded",
            r#"{"error":"overloaded"}"#,
            Failure::RateLimited,
        ),
        (
            "402 Payment Required",
            r#"{"detail":{"message":"over budget: sk-secret"}}"#,
            Failure::Unavailable,
        ),
        ("500 Internal Server Error", "{}", Failure::Unavailable),
        ("408 Request Timeout", "{}", Failure::Timeout),
        (
            "200 OK",
            r#"{"answers":{"q0":{"type":"score","noul":0.9}}}"#,
            Failure::Invalid,
        ),
        ("200 OK", "not json", Failure::Invalid),
        ("200 OK", r#"{"answers":{}}"#, Failure::Invalid),
        (
            "200 OK",
            r#"{"answers":{"q0":{"type":"noul","noul":1.4}}}"#,
            Failure::Invalid,
        ),
        (
            "200 OK",
            r#"{"answers":{"q0":{"type":"choice","choice":"yes"}}}"#,
            Failure::Invalid,
        ),
    ];
    for (status, body, expected) in cases {
        let (endpoint, server) = serve(status, body, Duration::ZERO);
        let replies = jev(endpoint, 5_000).answer_all(&[&commit_question()], &evidence());
        server.join().unwrap();
        assert_eq!(replies, [Err(expected)], "{status} {body}");
    }
}

#[test]
fn a_slow_reply_times_out_and_a_closed_port_is_unavailable() {
    let (endpoint, server) = serve("200 OK", LIVE_RESPONSE, Duration::from_millis(600));
    assert_eq!(
        jev(endpoint, 150).answer_all(&[&commit_question()], &evidence()),
        [Err(Failure::Timeout)]
    );
    drop(server);

    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    assert_eq!(
        jev(endpoint, 1_000).answer_all(&[&commit_question()], &evidence()),
        [Err(Failure::Unavailable)]
    );
}

#[test]
fn a_reply_over_the_size_bound_is_invalid() {
    let huge: &'static str = Box::leak(
        format!(r#"{{"answers":{{}},"pad":"{}"}}"#, "x".repeat(300_000)).into_boxed_str(),
    );
    let (endpoint, server) = serve("200 OK", huge, Duration::ZERO);

    let replies = jev(endpoint, 5_000).answer_all(&[&commit_question()], &evidence());
    server.join().unwrap();

    assert_eq!(replies, [Err(Failure::Invalid)]);
}

#[test]
fn a_question_that_is_not_yes_or_no_fails_as_invalid_without_a_call() {
    for labels in [["act", "ignore"], ["yes", "maybe"]] {
        let outcomes = Outcomes::new(labels).unwrap();
        let question = Question::new("What should the agent do?", outcomes, ["commit"]).unwrap();

        let replies = jev("http://127.0.0.1:9".into(), 1_000).answer_all(&[&question], &evidence());

        assert_eq!(replies, [Err(Failure::Invalid)], "{labels:?}");
    }
}
