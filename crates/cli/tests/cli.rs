//! The `chauffeur` binary as a person runs it: a daemon in the background,
//! then `emit`, `status` and `version` over its real socket, and `audit`
//! from the files the daemon writes.
#![expect(
    clippy::unwrap_used,
    reason = "the helpers fail the test on a broken fixture or output that is not JSON"
)]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;

const CHAUFFEUR: &str = env!("CARGO_BIN_EXE_chauffeur");

const NUDGING: &str = r#"{ "models": { "stand-in": { "format": "placeholder", "otherwise": { "yes": 0.94, "no": 0.06 } } },
                          "tiers": { "system-one": ["stand-in"] } }"#;

/// A private folder with the user's configuration, and the socket the
/// daemon and every command share. Stops the daemon when dropped.
struct Install {
    root: PathBuf,
    daemon: Option<Child>,
}

impl Install {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("chauffeur-cli-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("config/chauffeur/rules")).unwrap();
        fs::write(root.join("config/chauffeur/models.json"), NUDGING).unwrap();
        Self { root, daemon: None }
    }

    fn socket(&self) -> PathBuf {
        self.root.join("run/daemon.sock")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(CHAUFFEUR);
        self.environment(&mut command);
        command
    }

    fn environment(&self, command: &mut Command) {
        command
            .env("CHAUFFEUR_SOCKET", self.socket())
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("CHAUFFEUR_DISTRIBUTION", distribution());
    }

    /// Starts `chauffeur daemon` and waits for its socket.
    fn start(&mut self) {
        let mut daemon = self.command();
        daemon.arg("daemon");
        self.serve(daemon);
    }

    /// Spawns `daemon`, which runs `chauffeur daemon`, and waits for its socket.
    fn serve(&mut self, mut daemon: Command) {
        let log = self.root.join("daemon.log");
        let socket = self.socket();
        let child = self.daemon.insert(
            daemon
                .stdout(Stdio::null())
                .stderr(fs::File::create(&log).unwrap())
                .spawn()
                .unwrap(),
        );
        let started = Instant::now();
        while UnixStream::connect(&socket).is_err() {
            let exited = child.try_wait().unwrap();
            assert!(
                exited.is_none() && started.elapsed() < Duration::from_secs(10),
                "the daemon did not start ({exited:?}): {}",
                fs::read_to_string(&log).unwrap_or_default()
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Runs `chauffeur <args>` with `stdin`.
    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = self
            .command()
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    /// The runs `chauffeur audit` prints, one JSON line each.
    fn audit(&self) -> Vec<Value> {
        let output = self.run(&["audit"], "");
        assert!(output.status.success(), "audit failed: {output:?}");
        String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// Stops the daemon.
    fn stop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
    }

    /// Runs `chauffeur <args>`, which must succeed, and reads its JSON.
    fn json(&self, args: &[&str], stdin: &str) -> Value {
        let output = self.run(args, stdin);
        assert!(output.status.success(), "{args:?} failed: {output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

/// Answers one `/v1/systemone` call on loopback as Jev does, and returns
/// its endpoint and, once called, the authorization header it was sent.
fn jev_once() -> (String, thread::JoinHandle<String>) {
    const YES: &str = r#"{"model":"jev-1.13.0","answers":{"q0":{"type":"noul","noul":0.97}}}"#;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let (mut authorization, mut length) = (String::new(), 0);
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let lower = line.trim().to_ascii_lowercase();
            if lower.is_empty() {
                break;
            }
            if let Some(value) = lower.strip_prefix("content-length:") {
                length = value.trim().parse().unwrap();
            }
            if lower.starts_with("authorization:") {
                line.trim().clone_into(&mut authorization);
            }
        }
        reader.read_exact(&mut vec![0; length]).unwrap();
        let _ = write!(
            reader.get_mut(),
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{YES}",
            YES.len()
        );
        authorization
    });
    (endpoint, server)
}

fn distribution() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../distribution")
}

impl Drop for Install {
    fn drop(&mut self) {
        self.stop();
        let _ = fs::remove_dir_all(&self.root);
    }
}

const BAD_COMMIT: &str = r#"{ "event": "tool.after", "session": "s1", "tool": "shell",
                              "input": { "command": "git commit -m 'fixed stuff'" } }"#;

#[test]
fn a_bad_commit_sent_with_emit_gets_the_nudge_and_audit_and_status_show_it() {
    let mut install = Install::new("nudge");
    install.start();

    let emitted = install.json(&["emit", "--deadline-ms", "2000"], BAD_COMMIT);
    assert_eq!(emitted["actions"][0]["rule"], "conventional-commits");
    assert_eq!(emitted["actions"][0]["action"], "message.append");

    let runs = install.audit();
    let run = &runs[0];
    assert_eq!(run["rule"], "conventional-commits");
    assert_eq!(run["steps"][0]["answer"]["outcome"], "yes");
    assert_eq!(run["steps"][0]["answer"]["model"], "stand-in");

    let status = install.json(&["status"], "");
    assert_eq!(status["version"], env!("CARGO_PKG_VERSION"));
    assert!(
        status["rules"]
            .as_array()
            .unwrap()
            .contains(&Value::from("conventional-commits"))
    );
}

#[test]
fn a_second_daemon_finds_the_socket_taken_and_exits() {
    let mut install = Install::new("second");
    install.start();

    let second = install.run(&["daemon"], "");
    assert!(second.status.success());
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("a daemon is already running"),
        "{second:?}"
    );
}

#[test]
fn without_a_daemon_a_call_fails_and_names_the_socket() {
    let install = Install::new("none");

    let output = install.run(&["status"], "");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no daemon answers on"), "{stderr}");
    assert!(stderr.contains("daemon.sock"), "{stderr}");
}

#[test]
fn emit_refuses_an_event_that_is_not_json() {
    let mut install = Install::new("garbage");
    install.start();

    let output = install.run(&["emit"], "not json");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("the event on stdin is not JSON"));
}

#[test]
fn a_daemon_error_reaches_the_person_as_a_failure() {
    let mut install = Install::new("refused");
    install.start();

    let output = install.run(&["emit"], r#"{ "session": "s1", "tool": "shell" }"#);
    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("an event needs its name in `event`"),
        "{stderr}"
    );
}

#[test]
fn the_audit_log_outlasts_the_daemon_that_wrote_it() {
    let mut install = Install::new("durable");
    install.start();
    install.json(&["emit"], BAD_COMMIT);
    install.stop();

    let runs = install.audit();
    assert_eq!(runs.len(), 1, "read with no daemon running");
    assert_eq!(runs[0]["returned"], "message.append");

    install.start();
    install.json(&["emit"], BAD_COMMIT);
    let runs = install.audit();
    assert_eq!(runs.len(), 2, "a new daemon appends to the same log");
    assert_eq!(
        runs[1]["returned"], "message.append",
        "cooldowns start afresh with each daemon; the log only records"
    );
}

#[test]
fn audit_follow_brief_prints_each_new_run_on_one_line() {
    let mut install = Install::new("follow");
    install.start();
    let mut follower = install
        .command()
        .args(["audit", "0", "--follow", "--brief"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = follower.stdout.take().unwrap();
    let (send, lines) = std::sync::mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let _ = send.send(line.unwrap());
        }
    });
    // The follower starts reading from the log's end, so wait for it to start.
    thread::sleep(Duration::from_millis(500));

    install.json(&["emit"], BAD_COMMIT);
    let line = lines.recv_timeout(Duration::from_secs(5));
    let _ = follower.kill();
    let _ = follower.wait();

    let line = line.unwrap();
    let time: Vec<char> = line.chars().take(8).collect();
    assert!(
        time.iter()
            .enumerate()
            .all(|(at, c)| if at == 2 || at == 5 {
                *c == ':'
            } else {
                c.is_ascii_digit()
            }),
        "starts with the local time: {line}"
    );
    assert!(
        line.contains("conventional-commits  s1  nonconventional yes 0.94 stand-in (placeholder)"),
        "{line}"
    );
    assert!(line.ends_with("→ message.append"), "{line}");
}

#[test]
fn version_prints_the_installation_name_and_version() {
    let output = Install::new("version").run(&["version"], "");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("chauffeur {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn a_model_whose_key_is_set_answers_with_it() {
    let mut install = Install::new("keyed");
    let (endpoint, server) = jev_once();
    let models = format!(
        r#"{{ "models": {{ "jev": {{ "format": "systemone", "endpoint": "{endpoint}", "model": "jev-latest",
                                      "key": {{ "env": "CHAUFFEUR_TEST_KEY" }} }} }},
             "tiers": {{ "system-one": ["jev"] }} }}"#
    );
    fs::write(install.root.join("config/chauffeur/models.json"), models).unwrap();
    let mut daemon = install.command();
    daemon.arg("daemon").env("CHAUFFEUR_TEST_KEY", "test-key");
    install.serve(daemon);

    let emitted = install.json(&["emit", "--deadline-ms", "5000"], BAD_COMMIT);
    assert_eq!(emitted["actions"][0]["action"], "message.append");
    assert_eq!(server.join().unwrap(), "authorization: Bearer test-key");
    let step = &install.audit()[0]["steps"][0];
    assert_eq!(step["answer"]["model"], "jev");
    assert_eq!(step["answer"]["placeholder"], false);
}

/// macOS's sandbox, as `brew test` runs a formula's test, refuses `FSEvents`.
/// The daemon still starts, and a rule added afterwards still loads.
#[cfg(target_os = "macos")]
#[test]
fn without_fsevents_the_daemon_still_reloads_an_added_rule() {
    const NO_FSEVENTS: &str =
        r#"(version 1)(allow default)(deny mach-lookup (global-name "com.apple.FSEvents"))"#;
    let mut install = Install::new("sandboxed");
    let mut daemon = Command::new("sandbox-exec");
    daemon.args(["-p", NO_FSEVENTS, CHAUFFEUR, "daemon"]);
    install.environment(&mut daemon);
    install.serve(daemon);

    let rule = fs::read_to_string(distribution().join("rules/conventional-commits.json")).unwrap();
    fs::write(install.root.join("config/chauffeur/rules/added.json"), rule).unwrap();
    let added = Value::from("added");
    let started = Instant::now();
    while !install.json(&["status"], "")["rules"]
        .as_array()
        .unwrap()
        .contains(&added)
    {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the added rule never loaded"
        );
        thread::sleep(Duration::from_millis(100));
    }
}
