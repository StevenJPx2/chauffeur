//! The daemon through its real socket, with the repository's distribution
//! and a user configuration whose placeholder stands in for Jev.
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "the helpers fail the test on a broken fixture or a reply missing a field"
)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use chauffeur_daemon::{DaemonError, Paths};
use serde_json::{Value, json};

const NUDGING: &str = r#"{ "models": { "stand-in": { "format": "placeholder", "otherwise": { "yes": 0.94, "no": 0.06 } } },
                          "tiers": { "system-one": ["stand-in"] } }"#;

/// Fresh paths in a private folder of their own, with `models` as the
/// user's `models.json`.
fn paths(name: &str, models: &str) -> Paths {
    let root = std::env::temp_dir().join(format!("chauffeur-daemon-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("config/rules")).unwrap();
    fs::write(root.join("config/models.json"), models).unwrap();
    Paths {
        name: "chauffeur-test".into(),
        socket: root.join("run/daemon.sock"),
        config: root.join("config"),
        distribution: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../distribution"),
    }
}

/// Starts the daemon on its own thread and waits for its socket.
fn start(paths: &Paths) {
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
}

fn send(socket: &Path, line: &str) -> Value {
    let mut stream = UnixStream::connect(socket).unwrap();
    writeln!(stream, "{line}").unwrap();
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).unwrap();
    serde_json::from_str(&reply).unwrap()
}

fn call(socket: &Path, method: &str, params: &Value) -> Value {
    let reply = send(
        socket,
        &json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params }).to_string(),
    );
    assert_eq!(reply["id"], 7);
    reply["result"].clone()
}

fn emit(socket: &Path, session: &str, command: &str) -> Value {
    let event = json!({ "event": "tool.after", "session": session, "tool": "shell", "input": { "command": command } });
    call(socket, "chauffeur.emit", &json!({ "event": event }))["actions"].clone()
}

#[test]
fn a_bad_commit_gets_the_nudge_once_per_cooldown_and_the_log_shows_why() {
    let paths = paths("nudge", NUDGING);
    start(&paths);

    let actions = emit(&paths.socket, "s1", "git commit -m 'fixed stuff'");
    assert_eq!(actions[0]["rule"], "conventional-commits");
    assert_eq!(actions[0]["action"], "message.append");
    assert_eq!(actions[0]["payload"]["delivery"], "now");
    assert_eq!(
        emit(&paths.socket, "s1", "ls -la"),
        json!([]),
        "no rule hooks ls"
    );
    assert_eq!(
        emit(&paths.socket, "s1", "git commit -m 'more stuff'"),
        json!([]),
        "cooling down"
    );
    assert_eq!(
        emit(&paths.socket, "s2", "git commit -m 'more stuff'")[0]["action"],
        "message.append",
        "another session"
    );

    let runs = call(&paths.socket, "chauffeur.events", &Value::Null)["runs"].clone();
    let first = &runs[0]["steps"][0];
    assert_eq!(first["step"], "nonconventional");
    assert_eq!(first["answer"]["outcome"], "yes");
    assert_eq!(first["answer"]["model"], "stand-in");
    assert_eq!(first["answer"]["placeholder"], true);
    assert_eq!(runs[1]["cooldown"], true);
    assert_eq!(runs.as_array().unwrap().len(), 3, "ls ran no rule");
}

#[test]
fn with_no_model_to_ask_nothing_is_sent_and_the_log_says_unavailable() {
    let paths = paths("unavailable", r#"{ "tiers": { "system-one": [] } }"#);
    start(&paths);

    assert_eq!(
        emit(&paths.socket, "s1", "git commit -m 'fixed stuff'"),
        json!([])
    );

    let runs = call(&paths.socket, "chauffeur.events", &Value::Null)["runs"].clone();
    assert_eq!(runs[0]["steps"][0]["failed"], "unavailable");
}

#[test]
fn an_edited_rule_applies_to_the_next_event_and_a_bad_one_is_refused_with_its_path() {
    let paths = paths("reload", NUDGING);
    start(&paths);
    let shipped =
        fs::read_to_string(paths.distribution.join("rules/conventional-commits.json")).unwrap();
    let edited = shipped.replace("git commit", "jj describe");

    fs::write(paths.config.join("rules/conventional-commits.json"), edited).unwrap();
    fs::create_dir_all(paths.config.join("rules/team")).unwrap();
    fs::write(
        paths.config.join("rules/team/broken.json"),
        r#"{ "on": "x", "strategy": [{ "id": "s", "judge": "nobody", "ask": "a" }] }"#,
    )
    .unwrap();

    let started = Instant::now();
    while emit(&paths.socket, "s1", "jj describe -m 'fixed stuff'") == json!([]) {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the edit was not loaded"
        );
        thread::sleep(Duration::from_millis(100));
    }
    let status = call(&paths.socket, "chauffeur.status", &Value::Null);
    assert_eq!(
        emit(&paths.socket, "s9", "git commit -m 'fixed stuff'"),
        json!([]),
        "the user's rule replaced the shipped one"
    );
    let refused = status["refused"][0].as_str().unwrap();
    assert!(
        refused.contains("team/broken.json") && refused.contains("rule `team/broken`"),
        "{refused}"
    );
    assert_eq!(status["rules"], json!(["conventional-commits"]));
}

#[test]
fn malformed_and_oversized_requests_fail_safely() {
    let paths = paths("malformed", NUDGING);
    start(&paths);

    assert_eq!(send(&paths.socket, "not json")["error"]["code"], -32_700);
    assert_eq!(
        send(
            &paths.socket,
            r#"{ "id": 1, "method": "chauffeur.status" }"#
        )["error"]["code"],
        -32_600
    );
    let unknown = send(
        &paths.socket,
        r#"{ "jsonrpc": "2.0", "id": 1, "method": "chauffeur.nope" }"#,
    );
    assert_eq!(unknown["error"]["code"], -32_601);
    let no_event = send(
        &paths.socket,
        r#"{ "jsonrpc": "2.0", "id": 1, "method": "chauffeur.emit", "params": {} }"#,
    );
    assert_eq!(no_event["error"]["code"], -32_602);

    let oversized = "x".repeat(chauffeur_daemon::MAX_LINE.saturating_add(10));
    assert!(
        send(&paths.socket, &oversized)["error"]["message"]
            .as_str()
            .unwrap()
            .contains("over 1 MiB")
    );
    assert_eq!(
        call(&paths.socket, "chauffeur.status", &Value::Null)["protocol"],
        1,
        "the daemon still answers"
    );
}

#[test]
fn a_second_daemon_finds_the_first_and_an_open_folder_is_refused() {
    let paths = paths("second", NUDGING);
    start(&paths);

    assert!(matches!(
        chauffeur_daemon::run(&paths),
        Err(DaemonError::Running(_))
    ));

    let open = Paths {
        socket: paths.socket.with_file_name("open/daemon.sock"),
        ..paths
    };
    fs::create_dir_all(open.socket.parent().unwrap()).unwrap();
    fs::set_permissions(
        open.socket.parent().unwrap(),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let error = chauffeur_daemon::run(&open).unwrap_err();
    assert!(
        error.to_string().ends_with("must have mode 0700"),
        "{error}"
    );
}
