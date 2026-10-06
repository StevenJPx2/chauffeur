//! A private Chauffeur daemon for one run, killed when its guard drops.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::process::{ChildGuard, Exit};

const STARTUP: Duration = Duration::from_secs(15);
const POLL: Duration = Duration::from_millis(200);
const IO_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;

/// Where a run's daemon keeps state and reads its config and skills.
#[derive(Clone, Debug)]
pub struct DaemonDirs {
    pub state: PathBuf,
    /// The run's own config folder, holding the variant's overrides.
    pub config: PathBuf,
    /// The shipped skills under test.
    pub skills: PathBuf,
}

/// A running daemon. Dropping it kills the process.
pub struct Daemon {
    _child: ChildGuard,
    port: u16,
}

impl Daemon {
    /// Start `<bin> daemon --port <free port>` with its state in `dirs.state`,
    /// config and skills from `dirs` only, and output in `log`, and wait until
    /// it answers `health`.
    ///
    /// # Errors
    /// No free port, a daemon that cannot start, exits early, or is not
    /// healthy within 15 seconds.
    pub fn start(bin: &Path, dirs: &DaemonDirs, disable: &str, log: &Path) -> Result<Self, String> {
        let state_dir = &dirs.state;
        let port = free_port()?;
        for folder in [state_dir, &dirs.config] {
            std::fs::create_dir_all(folder)
                .map_err(|error| format!("create {}: {error}", folder.display()))?;
        }
        let out = std::fs::File::create(log)
            .map_err(|error| format!("create {}: {error}", log.display()))?;
        let err = out
            .try_clone()
            .map_err(|error| format!("open {}: {error}", log.display()))?;
        let mut child = ChildGuard::spawn(
            Command::new(bin)
                .args(["daemon", "--port", &port.to_string()])
                .env("CHAUFFEUR_STATE_DIR", state_dir)
                .env("CHAUFFEUR_CONFIG_DIR", &dirs.config)
                .env("CHAUFFEUR_SKILLS_DIR", &dirs.skills)
                .env("CHAUFFEUR_DISABLE", disable)
                .env("CHAUFFEUR_SOURCEFED", "off")
                .env("CHAUFFEUR_IDLE_STEERING", "true")
                .env_remove("CHAUFFEUR_DAEMON_TOKEN")
                .stdin(Stdio::null())
                .stdout(Stdio::from(out))
                .stderr(Stdio::from(err)),
            "chauffeur daemon",
        )?;

        wait_healthy(&mut child, port, log)?;

        Ok(Self {
            _child: child,
            port,
        })
    }

    /// The URL the plugin connects to.
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn wait_healthy(child: &mut ChildGuard, port: u16, log: &Path) -> Result<(), String> {
    let deadline = Instant::now() + STARTUP;

    while Instant::now() < deadline {
        if let Some(exit) = child.exited()? {
            return Err(format!(
                "chauffeur daemon exited during startup ({}): {}",
                describe(exit),
                log_tail(log)
            ));
        }
        if healthy(port) {
            return Ok(());
        }

        std::thread::sleep(POLL);
    }

    Err(format!(
        "chauffeur daemon on port {port} was not healthy within {}s: {}",
        STARTUP.as_secs(),
        log_tail(log)
    ))
}

fn describe(exit: Exit) -> String {
    match exit {
        Exit::Code(Some(code)) => format!("exit {code}"),
        Exit::Code(None) => "killed by a signal".into(),
        Exit::TimedOut => "timed out".into(),
    }
}

fn log_tail(log: &Path) -> String {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().rev().take(5).collect();

    lines.into_iter().rev().collect::<Vec<_>>().join(" | ")
}

/// A port that was free a moment ago: bind port 0 and release it.
///
/// # Errors
/// The loopback interface cannot be bound.
pub fn free_port() -> Result<u16, String> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| format!("find a free port: {error}"))
}

/// Whether the daemon on `port` answers `health` with `{"result":{"ok":true}}`.
#[must_use]
pub fn healthy(port: u16) -> bool {
    health_response(port).is_some_and(|body| is_healthy(&body))
}

fn health_response(port: u16) -> Option<String> {
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;
    let body = r#"{"id":1,"method":"health","params":{}}"#;
    let request = format!(
        "POST /rpc HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).ok()?;
    let mut response = String::new();
    stream
        .take(MAX_RESPONSE_BYTES)
        .read_to_string(&mut response)
        .ok()?;

    Some(response)
}

/// Whether a raw HTTP response is a 200 carrying a healthy result.
#[must_use]
pub fn is_healthy(response: &str) -> bool {
    let Some((head, body)) = response.split_once("\r\n\r\n") else {
        return false;
    };
    let ok_status = head
        .lines()
        .next()
        .is_some_and(|status| status.split_whitespace().nth(1) == Some("200"));

    ok_status
        && serde_json::from_str::<serde_json::Value>(body.trim())
            .ok()
            .and_then(|value| value.get("result")?.get("ok")?.as_bool())
            == Some(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_a_healthy_response() {
        let ok = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\r\n{\"id\":1,\"result\":{\"ok\":true}}";
        let bad = "HTTP/1.1 400 Bad Request\r\n\r\n{\"id\":1,\"error\":\"nope\"}";

        assert!(is_healthy(ok));
        assert!(!is_healthy(bad));
        assert!(!is_healthy("garbage"));
    }

    #[test]
    fn health_check_speaks_http_to_a_listener() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 1024];
            let read = stream.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).into_owned();
            let body = r#"{"id":1,"result":{"ok":true}}"#;
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
            request
        });

        assert!(healthy(port));
        let request = server.join().unwrap();
        assert!(request.starts_with("POST /rpc HTTP/1.1"));
        assert!(request.ends_with(r#"{"id":1,"method":"health","params":{}}"#));
    }
}
