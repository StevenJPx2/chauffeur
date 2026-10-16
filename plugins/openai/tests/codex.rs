//! The Codex backend against a fake Responses endpoint and token server on
//! loopback: request shape, each kind of outcome, and token refresh.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use chauffeur_core::{Backend, Completion, ReasoningFailure};
use chauffeur_plugin_openai::codex::{CodexAuth, CodexBackend};
use serde_json::{Value, json};

/// One request the fake server saw.
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// A reply: status, content type, body, and a delay before sending it.
type Reply = (u16, &'static str, String, Duration);

/// Serve `replies` in order, one per connection; returns the base URL and
/// what each request carried.
fn serve(replies: Vec<Reply>) -> (String, Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (seen, requests) = mpsc::channel();

    thread::spawn(move || {
        for (status, kind, body, delay) in replies {
            let Ok((stream, _)) = listener.accept() else {
                return;
            };
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let path = line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            let mut headers = Vec::new();
            let mut length = 0;

            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some((name, value)) = line.trim_end().split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                    headers.push((name.to_string(), value.trim().to_string()));
                }
            }

            let mut raw = vec![0; length];
            reader.read_exact(&mut raw).unwrap();
            let _ = seen.send(Seen {
                path,
                headers,
                body: serde_json::from_slice(&raw).unwrap_or(Value::Null),
            });

            thread::sleep(delay);
            let _ = write!(
                reader.get_mut(),
                "HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });

    (base, requests)
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// A JWT expiring at `exp`, named so tests can tell tokens apart.
fn jwt(name: &str, exp: u64) -> String {
    let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({ "exp": exp, "name": name }).to_string());

    format!("header.{claims}.signature")
}

/// An auth file in its own folder, mode 0600, with a field Chauffeur does
/// not know.
fn auth_file(test: &str, access: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("chauffeur-codex-{test}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("auth.json");
    let file = json!({
        "OPENAI_API_KEY": null,
        "auth_mode": "chatgpt",
        "last_refresh": "2026-01-01T00:00:00Z",
        "tokens": {
            "access_token": access,
            "refresh_token": "refresh-1",
            "id_token": "id-1",
            "account_id": "acct-7",
        },
        "something_new": { "kept": true },
    });

    std::fs::write(&path, file.to_string()).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();

    path
}

fn backend(path: PathBuf, base: &str) -> CodexBackend {
    let auth = CodexAuth::new(path).with_token_url(format!("{base}/oauth/token"));

    CodexBackend::new(Arc::new(auth))
        .unwrap()
        .with_endpoint(format!("{base}/backend-api/codex/responses"))
}

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "verdict": { "type": "string", "enum": ["complete", "continue"] },
            "cites": { "type": "array", "items": { "type": "string", "enum": [] }, "maxItems": 4 },
        },
        "required": ["verdict", "cites"],
        "additionalProperties": false,
    })
}

fn complete(backend: &mut CodexBackend, timeout: Duration) -> chauffeur_core::Reasoned {
    let schema = schema();

    backend.complete(
        &Completion {
            model: "gpt-6-luna",
            effort: Some("low"),
            instructions: "Judge the goal.",
            input: "Goal: tests pass.",
            name: "completion review",
            schema: &schema,
            max_output_tokens: 512,
            timeout,
        },
        &mut chauffeur_core::CompletionMetadata::default(),
    )
}

/// A Responses event stream of `events`.
fn stream(events: &[Value]) -> String {
    events
        .iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}

fn delta(text: &str) -> Value {
    json!({ "type": "response.output_text.delta", "delta": text })
}

fn completed() -> Value {
    json!({ "type": "response.completed", "response": { "status": "completed" } })
}

fn sse(events: &[Value]) -> Reply {
    (200, "text/event-stream", stream(events), Duration::ZERO)
}

const SECOND: Duration = Duration::from_secs(5);

#[test]
fn a_verdict_streams_back_as_json_from_a_request_shaped_for_codex() {
    let access = jwt("fresh", now() + 3_600);
    let path = auth_file("verdict", &access);
    let (base, seen) = serve(vec![sse(&[
        json!({ "type": "response.created" }),
        delta("{\"verdict\":\"comp"),
        delta("lete\",\"cites\":[]}"),
        completed(),
    ])]);
    let verdict = complete(&mut backend(path, &base), SECOND);

    assert_eq!(verdict, Ok(json!({ "verdict": "complete", "cites": [] })));

    let request = seen.recv().unwrap();
    let format = &request.body["text"]["format"];

    assert_eq!(request.path, "/backend-api/codex/responses");
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {access}").as_str())
    );
    assert_eq!(request.header("chatgpt-account-id"), Some("acct-7"));
    assert_eq!(request.header("originator"), Some("codex_cli_rs"));
    assert_eq!(request.body["model"], "gpt-6-luna");
    assert_eq!(request.body["instructions"], "Judge the goal.");
    assert_eq!(
        request.body["input"][0]["content"][0]["text"],
        "Goal: tests pass."
    );
    assert_eq!(request.body["reasoning"]["effort"], "low");
    assert_eq!(request.body["max_output_tokens"], 512);
    assert_eq!(
        (&request.body["store"], &request.body["stream"]),
        (&json!(false), &json!(true))
    );
    assert_eq!(
        (&format["strict"], &format["name"]),
        (&json!(true), &json!("completion_review"))
    );
    // An empty enum is dropped: strict mode rejects it.
    assert_eq!(
        format["schema"]["properties"]["cites"]["items"],
        json!({ "type": "string" })
    );
}

#[test]
fn each_way_a_call_can_fail_is_its_own_outcome() {
    let access = jwt("fresh", now() + 3_600);
    type Expected = fn(&ReasoningFailure) -> bool;

    let cases: Vec<(&str, Reply, Expected)> = vec![
        (
            "refused",
            sse(&[
                json!({ "type": "response.refusal.delta", "delta": "I can't" }),
                completed(),
            ]),
            |failure| *failure == ReasoningFailure::Refused,
        ),
        (
            "truncated",
            sse(&[
                delta("{\"verdict\":"),
                json!({ "type": "response.incomplete",
                        "response": { "incomplete_details": { "reason": "max_output_tokens" } } }),
            ]),
            |failure| *failure == ReasoningFailure::Truncated,
        ),
        (
            "usage limit",
            sse(
                &[json!({ "type": "response.failed", "response": { "error": {
                "code": "usage_limit_reached", "message": "You have hit your limit." } } })],
            ),
            |failure| matches!(failure, ReasoningFailure::Failed(why) if why.contains("rate limited")),
        ),
        (
            "429",
            (
                429,
                "application/json",
                r#"{"detail":"slow down"}"#.into(),
                Duration::ZERO,
            ),
            |failure| matches!(failure, ReasoningFailure::Failed(why) if why.contains("429") && why.contains("slow down")),
        ),
        (
            "not json",
            sse(&[delta("complete"), completed()]),
            |failure| matches!(failure, ReasoningFailure::Invalid(_)),
        ),
        (
            "cut off",
            sse(&[delta("{}")]),
            |failure| matches!(failure, ReasoningFailure::Failed(why) if why.contains("ended")),
        ),
    ];

    for (name, reply, expected) in cases {
        let (base, _seen) = serve(vec![reply]);
        let outcome = complete(&mut backend(auth_file(name, &access), &base), SECOND);

        assert!(outcome.as_ref().is_err_and(expected), "{name}: {outcome:?}");
    }
}

#[test]
fn a_slow_provider_times_out() {
    let access = jwt("fresh", now() + 3_600);
    let (base, _seen) = serve(vec![(
        200,
        "text/event-stream",
        stream(&[delta("{}"), completed()]),
        Duration::from_secs(2),
    )]);
    let outcome = complete(
        &mut backend(auth_file("slow", &access), &base),
        Duration::from_millis(300),
    );

    assert_eq!(outcome, Err(ReasoningFailure::TimedOut));
}

#[test]
fn completed_metadata_survives_valid_and_invalid_verdicts_and_refusals() {
    let access = jwt("fresh", now() + 3_600);
    let completion = json!({ "type": "response.completed", "response": {
        "model": "reported-model", "usage": { "input_tokens": 123, "output_tokens": 45 }
    } });
    let (base, _seen) = serve(vec![
        sse(&[delta("{}"), completion.clone()]),
        sse(&[delta("not JSON"), completion.clone()]),
        sse(&[json!({ "type": "response.refusal.done" }), completion]),
    ]);
    let mut backend = backend(auth_file("metadata", &access), &base);
    for expected in [None, Some("invalid"), Some("refused")] {
        let mut metadata = chauffeur_core::CompletionMetadata::default();
        let result = backend.complete(
            &Completion {
                model: "requested",
                effort: None,
                instructions: "Judge.",
                input: "Evidence.",
                name: "review",
                schema: &schema(),
                max_output_tokens: 512,
                timeout: SECOND,
            },
            &mut metadata,
        );
        match expected {
            None => assert!(result.is_ok()),
            Some("invalid") => assert!(matches!(result, Err(ReasoningFailure::Invalid(_)))),
            _ => assert_eq!(result, Err(ReasoningFailure::Refused)),
        }
        assert_eq!(metadata.reported_model.as_deref(), Some("reported-model"));
        assert_eq!(metadata.usage.input_tokens, Some(123));
        assert_eq!(metadata.usage.output_tokens, Some(45));
    }
}

#[test]
fn an_expiring_login_is_refreshed_once_and_saved_for_the_codex_cli() {
    let stale = jwt("stale", now() + 30);
    let renewed = jwt("renewed", now() + 3_600);
    let path = auth_file("refresh", &stale);
    let (base, seen) =
        serve(vec![
        (
            200,
            "application/json",
            json!({ "access_token": renewed, "refresh_token": "refresh-2", "id_token": "id-2" })
                .to_string(),
            Duration::ZERO,
        ),
        sse(&[delta("{\"verdict\":\"continue\",\"cites\":[]}"), completed()]),
        sse(&[delta("{\"verdict\":\"complete\",\"cites\":[]}"), completed()]),
    ]);
    let mut backend = backend(path.clone(), &base);

    assert!(complete(&mut backend, SECOND).is_ok());
    assert!(complete(&mut backend, SECOND).is_ok());

    let refresh = seen.recv().unwrap();
    let first = seen.recv().unwrap();
    let second = seen.recv().unwrap();

    assert_eq!(refresh.path, "/oauth/token");
    assert_eq!(refresh.body["grant_type"], "refresh_token");
    assert_eq!(refresh.body["refresh_token"], "refresh-1");
    for call in [&first, &second] {
        assert_eq!(
            call.header("authorization"),
            Some(format!("Bearer {renewed}").as_str())
        );
    }

    let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

    assert_eq!(saved["tokens"]["access_token"], renewed);
    assert_eq!(saved["tokens"]["refresh_token"], "refresh-2");
    assert_eq!(saved["tokens"]["id_token"], "id-2");
    assert_eq!(saved["tokens"]["account_id"], "acct-7");
    assert_eq!(saved["something_new"], json!({ "kept": true }));
    assert_ne!(saved["last_refresh"], "2026-01-01T00:00:00Z");
    #[cfg(unix)]
    assert_eq!(
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&path).unwrap().permissions())
            & 0o777,
        0o600
    );
}

#[test]
fn a_rejected_token_is_read_again_from_the_file() {
    let old = jwt("old", now() + 3_600);
    let new = jwt("new", now() + 3_600);
    let path = auth_file("rejected", &old);
    let (base, seen) = serve(vec![
        (401, "application/json", "{}".into(), Duration::ZERO),
        sse(&[
            delta("{\"verdict\":\"complete\",\"cites\":[]}"),
            completed(),
        ]),
    ]);
    let mut backend = backend(path.clone(), &base);

    assert!(complete(&mut backend, SECOND).is_err());

    // The Codex CLI logs in again meanwhile.
    let mut file: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    file["tokens"]["access_token"] = new.clone().into();
    std::fs::write(&path, file.to_string()).unwrap();

    assert!(complete(&mut backend, SECOND).is_ok());
    seen.recv().unwrap();
    assert_eq!(
        seen.recv().unwrap().header("authorization"),
        Some(format!("Bearer {new}").as_str())
    );
}

#[test]
fn without_a_login_the_backend_is_unavailable() {
    let (base, _seen) = serve(Vec::new());
    let missing = std::env::temp_dir().join("chauffeur-codex-none/auth.json");
    let outcome = complete(&mut backend(missing, &base), SECOND);

    assert!(
        matches!(&outcome, Err(ReasoningFailure::Unavailable(why)) if why.contains("codex login")),
        "{outcome:?}"
    );
}

/// Against your real Codex login and the real endpoint. Run with
/// `cargo test -p chauffeur-plugin-openai -- --ignored`.
#[test]
#[ignore = "calls the real Codex endpoint with your login"]
fn live_luna_returns_a_verdict_that_fits_the_schema() {
    let auth = CodexAuth::new(CodexAuth::default_path().unwrap());
    let mut backend = CodexBackend::new(Arc::new(auth)).unwrap();
    let schema = json!({
        "type": "object",
        "properties": {
            "verdict": { "type": "string", "enum": ["complete", "continue"] },
            "cites": { "type": "array", "items": { "type": "string", "enum": ["tests"] }, "maxItems": 4 },
        },
        "required": ["verdict", "cites"],
        "additionalProperties": false,
    });
    let started = std::time::Instant::now();
    let verdict = backend.complete(&Completion {
        model: "gpt-6-luna",
        effort: Some("low"),
        instructions: "Judge whether the goal is done from the evidence. Cite evidence by id.",
        input: "Goal: make the tests pass.\n<evidence id=\"tests\">\ncargo test: 41 passed, 0 failed\n</evidence>",
        name: "completion-review",
        schema: &schema,
        max_output_tokens: 512,
        timeout: Duration::from_secs(10),
    }, &mut chauffeur_core::CompletionMetadata::default());

    println!("{:?} {verdict:?}", started.elapsed());
    assert_eq!(
        verdict.unwrap(),
        json!({ "verdict": "complete", "cites": ["tests"] })
    );
}
