//! Contract test against a local HTTP fixture that replays a response
//! captured from the live Jev API (jev-1.13.0).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chauffeur_core::{AnswerValue, ChoiceOption, Question, QuestionKind, SystemOne};
use chauffeur_judge_jev::{JevClient, JevConfig};

const LIVE_RESPONSE: &str = r#"{"model":"jev-1.13.0","answers":{"pick":{"type":"choice","choice":"openai/gpt-6-sol","confidence":0.53,"probabilities":{"openai/gpt-6-sol":0.76,"stay":0.24}},"ok":{"type":"noul","noul":0.97},"lvl":{"type":"score","score":1.01,"confidence":0.54,"legend":{"0":"low","1":"medium","2":"high"},"probabilities":{"0":0.15,"1":0.69,"2":0.16}}},"usage":{"input_tokens":397,"output_tokens":73}}"#;

fn questions() -> Vec<Question> {
    vec![
        Question {
            id: "pick".into(),
            instructions: "Which model?".into(),
            kind: QuestionKind::Choice {
                options: vec![
                    ChoiceOption {
                        value: "openai/gpt-6-sol".into(),
                        description: "frontier".into(),
                    },
                    ChoiceOption {
                        value: "stay".into(),
                        description: "do not switch".into(),
                    },
                ],
            },
        },
        Question {
            id: "ok".into(),
            instructions: "Coding task?".into(),
            kind: QuestionKind::Noul,
        },
        Question {
            id: "lvl".into(),
            instructions: "Urgency?".into(),
            kind: QuestionKind::Score {
                levels: vec!["low".into(), "medium".into(), "high".into()],
            },
        },
    ]
}

/// Serve one request; return (path, authorization, body) to the test.
fn serve(
    status: &'static str,
    body: &'static str,
) -> (String, JoinHandle<(String, String, String)>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind fixture");
    let base_url = format!("http://{}", listener.local_addr().expect("address"));
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream);
        let mut request_line = String::new();
        let mut authorization = String::new();
        let mut length = 0;

        reader.read_line(&mut request_line).expect("request line");

        loop {
            let mut line = String::new();
            reader.read_line(&mut line).expect("header");

            if line.trim().is_empty() {
                break;
            }

            let lower = line.to_ascii_lowercase();

            if let Some(value) = lower.strip_prefix("content-length:") {
                length = value.trim().parse().expect("length");
            }
            if lower.starts_with("authorization:") {
                authorization = line.trim().to_string();
            }
        }

        let mut request = vec![0; length];
        reader.read_exact(&mut request).expect("body");
        write!(
            reader.get_mut(),
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("respond");

        let path = request_line
            .split_whitespace()
            .nth(1)
            .unwrap_or_default()
            .to_string();

        (
            path,
            authorization,
            String::from_utf8(request).expect("utf8"),
        )
    });

    (base_url, server)
}

fn client(base_url: String) -> JevClient {
    JevClient::new(JevConfig {
        base_url,
        api_key: "test-key".into(),
        model: "jev-latest".into(),
        timeout: Duration::from_secs(5),
    })
    .expect("client")
}

#[test]
fn sends_one_request_and_parses_the_live_answer_shape() {
    let (base_url, server) = serve("200 OK", LIVE_RESPONSE);
    // The engine redacts before handing state and questions to the provider.
    let state = "agent hit a limit; leaked [REDACTED] in a log";
    let mut answers = client(base_url).ask(state, &questions()).expect("answers");
    let (path, authorization, body) = server.join().expect("fixture");
    let body: serde_json::Value = serde_json::from_str(&body).expect("json body");

    answers.sort_by(|a, b| a.id.cmp(&b.id));

    assert_eq!(path, "/v1/systemone");
    assert_eq!(authorization, "authorization: Bearer test-key");
    assert_eq!(body["model"], "jev-latest");
    assert_eq!(
        body["questions"]["pick"]["criteria"]["stay"],
        "do not switch"
    );
    assert_eq!(body["questions"]["lvl"]["criteria"][2], "high");
    assert!(body["questions"]["ok"].get("criteria").is_none());
    assert!(
        !body["state"].as_str().unwrap().contains("AKIA"),
        "secrets are redacted"
    );
    assert_eq!(answers[0].value, AnswerValue::Score(1.01));
    assert_eq!(answers[1].value, AnswerValue::Noul(0.97));
    assert_eq!(answers[1].confidence, None);
    assert_eq!(
        answers[2].value,
        AnswerValue::Choice("openai/gpt-6-sol".into())
    );
    assert_eq!(answers[2].confidence, Some(0.53));
}

#[test]
fn http_errors_and_mismatched_answers_fail() {
    let (base_url, server) = serve("529 Overloaded", r#"{"error":"overloaded"}"#);

    assert!(client(base_url).ask("s", &questions()).is_err());
    server.join().expect("fixture");

    let (base_url, server) = serve(
        "200 OK",
        r#"{"answers":{"pick":{"type":"choice","choice":"unknown"}}}"#,
    );

    assert!(client(base_url).ask("s", &questions()).is_err());
    server.join().expect("fixture");
}
