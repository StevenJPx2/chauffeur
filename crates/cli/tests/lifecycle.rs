//! The daemon reports its version and stops on `shutdown`, as a host
//! replacing a daemon of another version relies on.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// Kills the daemon if the test fails before it exits.
struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// One RPC over plain HTTP; the reply body, or `None` when nothing answers.
fn rpc(port: u16, method: &str) -> Option<serde_json::Value> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).ok()?;
    let body = format!(r#"{{"id":1,"method":"{method}","params":{{}}}}"#);
    let request = format!(
        "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;

    serde_json::from_str(response.split("\r\n\r\n").nth(1)?).ok()
}

fn until(deadline: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();

    while start.elapsed() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    false
}

#[test]
fn health_reports_the_version_and_shutdown_stops_the_daemon() {
    let dir = std::env::temp_dir().join(format!("chauffeur-lifecycle-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config")).unwrap();
    let skills = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills");
    let port = free_port();

    let mut daemon = Daemon(
        Command::new(env!("CARGO_BIN_EXE_chauffeur"))
            .args(["daemon", "--port", &port.to_string()])
            // Never called: the test sends no signal.
            .env("TYPESAFE_API_KEY", "test")
            .env("CHAUFFEUR_STATE_DIR", dir.join("state"))
            .env("CHAUFFEUR_CONFIG_DIR", dir.join("config"))
            .env("CHAUFFEUR_SKILLS_DIR", skills)
            .env("CHAUFFEUR_SOURCEFED", "off")
            .env_remove("CHAUFFEUR_DAEMON_TOKEN")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );

    assert!(
        until(Duration::from_secs(15), || rpc(port, "health").is_some()),
        "daemon never answered"
    );
    let health = rpc(port, "health").unwrap();
    assert_eq!(health["result"]["version"], env!("CARGO_PKG_VERSION"));

    assert_eq!(rpc(port, "shutdown").unwrap()["result"]["ok"], true);
    assert!(
        until(Duration::from_secs(5), || daemon
            .0
            .try_wait()
            .unwrap()
            .is_some()),
        "daemon still running after shutdown"
    );
    assert!(rpc(port, "health").is_none());

    let _ = std::fs::remove_dir_all(&dir);
}
