//! `chauffeur audit` under a reader that stops early, as `head` does.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// A state folder of its own, removed when dropped.
struct State(PathBuf);

impl State {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("chauffeur-audit-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    /// Appends `records` decisions to the audit log, each a long line.
    fn log(&self, records: usize) {
        let record = serde_json::json!({
            "at": 3_661,
            "agent_id": "ses_test",
            "signal": "tool_result",
            "detail": "x".repeat(200),
            "effects": [],
            "trace": { "elapsed_ms": 1, "questions": [] }
        });
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.0.join("audit.jsonl"))
            .unwrap()
            .write_all(format!("{record}\n").repeat(records).as_bytes())
            .unwrap();
    }

    fn audit(&self, args: &[&str]) -> Child {
        Command::new(env!("CARGO_BIN_EXE_chauffeur"))
            .arg("audit")
            .args(args)
            .env("CHAUFFEUR_STATE_DIR", &self.0)
            .env("CHAUFFEUR_CONFIG_DIR", self.0.join("config"))
            .env("CHAUFFEUR_SOURCEFED", "off")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap()
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Reads one line from `child`'s stdout, then closes the pipe.
fn first_line(child: &mut Child) -> String {
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    line
}

fn stderr(child: &mut Child) -> String {
    let mut text = String::new();
    child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

#[test]
fn audit_stops_quietly_when_its_reader_closes_the_pipe() {
    let state = State::new("closed-pipe");
    // About 250 KB printed: far more than a pipe holds, so audit is still
    // printing when it closes.
    state.log(1_000);
    let mut audit = state.audit(&["1000"]);

    assert!(first_line(&mut audit).contains("tool_result"));
    let status = audit.wait().unwrap();
    let stderr = stderr(&mut audit);

    assert!(status.success(), "{status:?}: {stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}

#[test]
fn audit_follow_stops_quietly_when_its_reader_closes_the_pipe() {
    let state = State::new("closed-follow");
    state.log(1);
    let mut follower = state.audit(&["1", "--follow"]);

    assert!(first_line(&mut follower).contains("tool_result"));
    // Printed once the reader has gone.
    state.log(1);
    let started = Instant::now();
    let status = loop {
        if let Some(status) = follower.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > Duration::from_secs(5) {
            let _ = follower.kill();
            let _ = follower.wait();
            panic!("still following");
        }
        thread::sleep(Duration::from_millis(50));
    };
    let stderr = stderr(&mut follower);

    assert!(status.success(), "{status:?}: {stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}
