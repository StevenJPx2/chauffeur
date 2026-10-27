//! The `chauffeur` binary as a person runs it: a daemon in the background,
//! then `emit`, `events`, `status` and `version` over its real socket.
#![expect(
    clippy::unwrap_used,
    reason = "the helpers fail the test on a broken fixture or output that is not JSON"
)]

use std::fs;
use std::io::Write;
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
        command
            .env("CHAUFFEUR_SOCKET", self.socket())
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env(
                "CHAUFFEUR_DISTRIBUTION",
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../distribution"),
            );
        command
    }

    /// Starts `chauffeur daemon` and waits for its socket.
    fn start(&mut self) {
        let child = self
            .command()
            .arg("daemon")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.daemon = Some(child);
        let started = Instant::now();
        while UnixStream::connect(self.socket()).is_err() {
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "the daemon did not start"
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

    /// Runs `chauffeur <args>`, which must succeed, and reads its JSON.
    fn json(&self, args: &[&str], stdin: &str) -> Value {
        let output = self.run(args, stdin);
        assert!(output.status.success(), "{args:?} failed: {output:?}");
        serde_json::from_slice(&output.stdout).unwrap()
    }
}

impl Drop for Install {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn a_bad_commit_sent_with_emit_gets_the_nudge_and_events_and_status_show_it() {
    let mut install = Install::new("nudge");
    install.start();

    let event = r#"{ "event": "tool.after", "session": "s1", "tool": "shell",
                     "input": { "command": "git commit -m 'fixed stuff'" } }"#;
    let emitted = install.json(&["emit", "--deadline-ms", "2000"], event);
    assert_eq!(emitted["actions"][0]["rule"], "conventional-commits");
    assert_eq!(emitted["actions"][0]["action"], "message.append");

    let events = install.json(&["events"], "");
    let run = &events["runs"][0];
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
fn version_prints_the_installation_name_and_version() {
    let output = Install::new("version").run(&["version"], "");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        format!("chauffeur {}", env!("CARGO_PKG_VERSION"))
    );
}
