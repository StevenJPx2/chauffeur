//! SubQ defaults and response contract, without paid calls or real credentials.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use chauffeur_core::{AnswerValue, ChoiceOption, Question, QuestionKind, SystemOne};
use chauffeur_judge_subq::{DEFAULT_MODEL, SubqClient, SubqConfig};
use serde_json::{Value, json};

fn fixture(status: &str, reply: String) -> (SubqClient, thread::JoinHandle<Value>) {
    delayed_fixture(status, reply, Duration::from_secs(3), Duration::ZERO)
}

fn delayed_fixture(
    status: &str,
    reply: String,
    timeout: Duration,
    delay: Duration,
) -> (SubqClient, thread::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let status = status.to_string();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "POST /v1/systemone HTTP/1.1");
        let mut length = 0;
        let mut authorized = false;
        loop {
            line.clear();
            reader.read_line(&mut line).unwrap();
            if line.trim().is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once(':') {
                if key.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse().unwrap();
                }
                if key.eq_ignore_ascii_case("authorization") {
                    authorized = value.trim() == "Bearer subq-test-key";
                }
            }
        }
        assert!(authorized);
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        thread::sleep(delay);
        // The size-cap test may close the socket before consuming the body.
        let _ = write!(
            reader.get_mut(),
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        );
        serde_json::from_slice(&body).unwrap()
    });
    let client = SubqClient::new(SubqConfig {
        base_url: url,
        api_key: "subq-test-key".into(),
        model: DEFAULT_MODEL.into(),
        timeout,
        record: None,
    })
    .unwrap();
    (client, server)
}

fn questions() -> Vec<Question> {
    vec![
        Question {
            id: "rules/check".into(),
            instructions: "Needed?".into(),
            kind: QuestionKind::Noul,
        },
        Question {
            id: "skill-exposure/pick".into(),
            instructions: "Pick a skill".into(),
            kind: QuestionKind::Choice {
                options: vec![
                    ChoiceOption {
                        value: "cli".into(),
                        description: "Use CLI".into(),
                    },
                    ChoiceOption {
                        value: "none".into(),
                        description: "No skill".into(),
                    },
                ],
            },
        },
        Question {
            id: "urgency".into(),
            instructions: "Urgency".into(),
            kind: QuestionKind::Score {
                levels: vec!["routine".into(), "urgent".into()],
            },
        },
    ]
}

#[test]
fn subq_sends_its_model_and_preserves_chauffeur_question_ids() {
    let reply = json!({"model":DEFAULT_MODEL,"answers":{
        "rules/check":{"type":"noul","noul":0.92},
        "skill-exposure/pick":{"type":"choice","choice":"cli","confidence":0.8,"probabilities":{"cli":0.9,"none":0.1}},
        "urgency":{"type":"score","score":0.75,"confidence":0.5,"legend":{"0":"routine","1":"urgent"},"probabilities":{"0":0.25,"1":0.75}}
    },"usage":{"input_tokens":100,"output_tokens":0}}).to_string();
    let (mut client, server) = fixture("200 OK", reply);
    assert_eq!(client.name(), "subq");
    let answers = client.ask("redacted [REDACTED]", &questions()).unwrap();
    let request = server.join().unwrap();
    assert_eq!(request["model"], DEFAULT_MODEL);
    assert_eq!(request["state"], "redacted [REDACTED]");
    assert_eq!(
        request["questions"]["skill-exposure/pick"]["criteria"]["cli"],
        "Use CLI"
    );
    assert_eq!(request["questions"]["urgency"]["criteria"][1], "urgent");
    assert_eq!(
        answers
            .iter()
            .find(|a| a.id == "rules/check")
            .unwrap()
            .value,
        AnswerValue::Noul(0.92)
    );
    assert_eq!(
        answers.iter().find(|a| a.id == "urgency").unwrap().value,
        AnswerValue::Score(0.75)
    );
}

#[test]
fn invalid_and_oversized_replies_fail_without_echoing_response_contents() {
    for (status, reply) in [
        (
            "402 Payment Required",
            "{\"secret\":\"do-not-log-response\"}".into(),
        ),
        ("200 OK", "not JSON".into()),
        ("200 OK", "{\"answers\":{}}".into()),
        ("200 OK", "x".repeat(262_145)),
    ] {
        let (mut client, server) = fixture(status, reply);
        let error = client.ask("state", &questions()).unwrap_err();
        assert!(!error.0.contains("do-not-log-response"));
        server.join().unwrap();
    }
}

#[test]
fn a_slow_provider_returns_an_error_within_its_timeout() {
    let (mut client, server) = delayed_fixture(
        "200 OK",
        "{}".into(),
        Duration::from_millis(50),
        Duration::from_millis(250),
    );
    let start = std::time::Instant::now();
    assert!(client.ask("state", &questions()).is_err());
    assert!(start.elapsed() < Duration::from_secs(1));
    server.join().unwrap();
}
