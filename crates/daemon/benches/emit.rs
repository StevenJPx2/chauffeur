//! One `chauffeur.emit` through the daemon's real socket, with the shipped
//! rules and a stand-in model: connecting, routing, running the rule and
//! writing the audit log. The daemon's threads allocate outside the count,
//! so these report time only.
#![expect(
    clippy::unwrap_used,
    reason = "a broken fixture or a daemon that will not start stops the benchmark"
)]

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use benchfmt::Suite;
use chauffeur_daemon::Paths;

const STAND_IN: &str = r#"{ "models": { "stand-in": { "format": "placeholder",
                                                     "otherwise": { "yes": 0.94, "no": 0.06 } } },
                            "tiers": { "system-one": ["stand-in"] } }"#;

/// A daemon on its own thread, in a fresh folder, once its socket answers.
fn start(root: &Path) -> Paths {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root.join("config/rules")).unwrap();
    fs::write(root.join("config/models.json"), STAND_IN).unwrap();
    let paths = Paths {
        name: "chauffeur-bench".into(),
        socket: root.join("run/daemon.sock"),
        config: root.join("config"),
        distribution: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../distribution"),
        state: root.join("state"),
    };
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
    paths
}

/// Emits a shell call in `session` and returns the reply line.
fn emit(socket: &Path, session: u64, command: &str) -> String {
    let event = serde_json::json!({
        "event": "tool.after", "session": format!("ses_{session}"), "tool": "shell",
        "input": { "command": command }
    });
    let request = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "chauffeur.emit", "params": { "event": event }
    });
    let mut stream = UnixStream::connect(socket).unwrap();
    writeln!(stream, "{request}").unwrap();
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).unwrap();
    reply
}

fn main() {
    let root = std::env::temp_dir().join(format!("chauffeur-bench-emit-{}", std::process::id()));
    let paths = start(&root);
    let socket = paths.socket.as_path();
    let mut session = 0_u64;

    Suite::from_env("chauffeur-daemon")
        .bench("Emit/rule=none", |b| {
            b.iter(|| emit(socket, 0, "ls -la"));
        })
        .bench("Emit/rule=nudge", |b| {
            // A fresh session each time, so the rule's cooldown never skips it.
            b.iter(|| {
                session = session.wrapping_add(1);
                emit(socket, session, "git commit -m 'fixed stuff'")
            });
        })
        .run();

    let _ = fs::remove_dir_all(&root);
}
