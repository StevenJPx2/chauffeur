//! Fastino's documented wire differences, without credentials or paid calls.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

use chauffeur_core::{AnswerValue, ChoiceOption, Facts, Question, QuestionKind, SystemOne};
use chauffeur_judge_glide::{DEFAULT_MODEL, GlideClient, GlideConfig};
use serde_json::{Value, json};

fn fixture(reply: Value) -> (GlideClient, thread::JoinHandle<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
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
                    authorized = value.trim() == "Bearer fake-fastino-key";
                }
            }
        }
        assert!(authorized);
        let mut body = vec![0; length];
        reader.read_exact(&mut body).unwrap();
        let reply = reply.to_string();
        write!(reader.get_mut(), "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}", reply.len()).unwrap();
        serde_json::from_slice(&body).unwrap()
    });
    let client = GlideClient::new(GlideConfig {
        base_url: url,
        api_key: "fake-fastino-key".into(),
        model: DEFAULT_MODEL.into(),
        timeout: Duration::from_secs(3),
        record: None,
    })
    .unwrap();
    (client, server)
}

fn question(kind: QuestionKind) -> Question {
    Question {
        id: "rules/check".into(),
        instructions: "Judge this evidence".into(),
        kind,
        facts: Facts::new(),
    }
}

#[test]
fn score_uses_expected_level_and_disables_provider_storage() {
    let (mut client, server) = fixture(json!({"answers": {
        "rules/check": {"type":"score", "score":0, "expected_level":0.375, "confidence":0.5}
    }}));
    let answers = client
        .ask(
            "redacted",
            &[question(QuestionKind::Score {
                levels: vec!["low".into(), "high".into()],
            })],
        )
        .unwrap();
    assert_eq!(client.name(), "glide");
    assert_eq!(answers[0].value, AnswerValue::Score(0.375));
    let request = server.join().unwrap();
    assert_eq!(request["model"], DEFAULT_MODEL);
    assert_eq!(request["store"], false);
    assert_eq!(request["state"], "redacted");
    assert_eq!(
        request["questions"]["rules/check"]["criteria"],
        json!(["low", "high"])
    );
}

#[test]
fn score_without_expected_level_or_outside_range_fails_closed() {
    for answer in [
        json!({"type":"score", "score":0}),
        json!({"type":"score", "score":0, "expected_level":2.0}),
        json!({"type":"score", "score":0, "expected_level":0.2, "confidence":1.1}),
    ] {
        let (mut client, server) = fixture(json!({"answers":{"rules/check":answer}}));
        assert!(
            client
                .ask(
                    "state",
                    &[question(QuestionKind::Score {
                        levels: vec!["low".into(), "high".into()],
                    })]
                )
                .is_err()
        );
        server.join().unwrap();
    }
}

#[test]
fn noul_and_choice_keep_ids_values_and_confidence() {
    for (kind, answer, expected) in [
        (
            QuestionKind::Noul,
            json!({"type":"noul","noul":0.9,"confidence":0.8}),
            AnswerValue::Noul(0.9),
        ),
        (
            QuestionKind::Choice {
                options: vec![
                    ChoiceOption {
                        value: "yes".into(),
                        description: "Needed".into(),
                    },
                    ChoiceOption {
                        value: "no".into(),
                        description: "Not needed".into(),
                    },
                ],
            },
            json!({"type":"choice","choice":"yes","confidence":0.8}),
            AnswerValue::Choice("yes".into()),
        ),
    ] {
        let (mut client, server) = fixture(json!({"answers":{"rules/check":answer}}));
        let answers = client.ask("state", &[question(kind)]).unwrap();
        assert_eq!(answers[0].id, "rules/check");
        assert_eq!(answers[0].value, expected);
        assert_eq!(answers[0].confidence, Some(0.8));
        server.join().unwrap();
    }
}
