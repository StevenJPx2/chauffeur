use std::io::{self, BufRead, BufReader, Read, Write};
use std::mem::DropGuard;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use crate::DaemonError;
use crate::paths::private_folder;
use crate::route::Router;
use crate::rpc;

/// The longest request line accepted.
pub const MAX_LINE: usize = 1 << 20;
/// The most connections served at once.
pub const MAX_CONNECTIONS: usize = 64;
/// How long telling a caller the daemon is busy may take.
const BUSY_WRITE: Duration = Duration::from_millis(100);
/// How long a failed accept waits before the next. An error that lasts,
/// such as running out of file descriptors, leaves the connection waiting,
/// so accepting again at once would fail at once, over and over.
const ACCEPT_RETRY: Duration = Duration::from_millis(50);

/// Binds the socket in its private folder. A socket another daemon answers
/// on is left alone; a stale one is replaced.
///
/// # Errors
///
/// [`DaemonError::Running`] when another daemon answers, or the folder or
/// socket cannot be set up.
pub fn bind(socket: &Path) -> Result<UnixListener, DaemonError> {
    private_folder(socket)?;
    if socket.exists() {
        if UnixStream::connect(socket).is_ok() {
            return Err(DaemonError::Running(socket.to_owned()));
        }
        std::fs::remove_file(socket).map_err(|source| DaemonError::Socket {
            path: socket.to_owned(),
            source,
        })?;
    }
    UnixListener::bind(socket).map_err(|source| DaemonError::Socket {
        path: socket.to_owned(),
        source,
    })
}

/// Serves connections for as long as the process runs, each on its own
/// thread. A failed accept is tried again after [`ACCEPT_RETRY`]. A
/// connection past [`MAX_CONNECTIONS`] is told the daemon is busy and
/// closed.
pub fn serve(listener: &UnixListener, router: &Arc<Router>) -> ! {
    let open = Arc::new(AtomicUsize::new(0));
    loop {
        let stream = next(|| listener.accept().map(|(stream, _)| stream));
        let taken = open.fetch_add(1, Ordering::SeqCst);
        // Freed however the connection ends, a panic included.
        let slot = DropGuard::new(Arc::clone(&open), |open| {
            open.fetch_sub(1, Ordering::SeqCst);
        });
        if taken >= MAX_CONNECTIONS {
            drop(slot);
            busy(stream);
            continue;
        }
        let router = Arc::clone(router);
        // A thread that cannot start drops this closure, and the slot with it.
        let _ = thread::Builder::new()
            .name("connection".into())
            .spawn(move || {
                connection(&router, stream);
                drop(slot);
            });
    }
}

/// The next connection `accept` gives. A failed accept waits
/// [`ACCEPT_RETRY`] before the next.
fn next<S>(mut accept: impl FnMut() -> io::Result<S>) -> S {
    loop {
        if let Ok(stream) = accept() {
            return stream;
        }
        thread::sleep(ACCEPT_RETRY);
    }
}

/// Tells a caller past the limit why it is closed, so it does not take the
/// closed connection for a daemon that stopped. A caller that is not reading
/// is not waited for.
fn busy(mut stream: UnixStream) {
    let reply = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32_000,
        "message": format!("the daemon is busy: it serves {MAX_CONNECTIONS} connections already") } });
    if stream.set_write_timeout(Some(BUSY_WRITE)).is_ok() {
        let _ = send(&mut stream, &reply);
    }
}

/// Answers each request line in turn. An oversized line gets an error and
/// closes the connection.
fn connection(router: &Router, stream: UnixStream) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    loop {
        line.clear();
        let limit = u64::try_from(MAX_LINE)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        match reader.by_ref().take(limit).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) if line.len() > MAX_LINE => {
                let reply = json!({ "jsonrpc": "2.0", "id": null,
                                    "error": { "code": -32_600, "message": "the request line is over 1 MiB" } });
                let _ = send(&mut writer, &reply);
                return;
            }
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        if text.trim().is_empty() {
            continue;
        }
        if let Some(reply) = rpc::handle(router, text.trim())
            && send(&mut writer, &reply).is_err()
        {
            return;
        }
    }
}

/// Writes `reply` and its newline in one write. Formatted straight to the
/// socket, a reply would go out in many small writes, each waking the
/// caller.
fn send(writer: &mut impl Write, reply: &Value) -> io::Result<()> {
    let mut line = reply.to_string();
    line.push('\n');
    writer.write_all(line.as_bytes())
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::time::Instant;

    use super::{ACCEPT_RETRY, next};

    /// An error that lasts, such as `EMFILE`, fails every accept at once;
    /// waiting between them keeps the loop from spinning a core.
    #[test]
    fn each_failed_accept_waits_before_the_next() {
        let mut calls = 0_u32;
        let started = Instant::now();

        let accepted = next(|| {
            calls = calls.saturating_add(1);
            if calls <= 3 {
                Err(io::Error::other("too many open files"))
            } else {
                Ok(calls)
            }
        });

        assert_eq!(accepted, 4, "accepted on the fourth try");
        assert!(
            started.elapsed() >= ACCEPT_RETRY.saturating_mul(3),
            "{:?} for three failed accepts",
            started.elapsed()
        );
    }

    #[test]
    fn an_accept_that_succeeds_does_not_wait() {
        let started = Instant::now();

        let accepted = next(|| io::Result::Ok("connection"));

        assert_eq!(accepted, "connection");
        assert!(started.elapsed() < ACCEPT_RETRY, "{:?}", started.elapsed());
    }
}
