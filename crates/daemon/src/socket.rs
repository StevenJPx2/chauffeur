use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use serde_json::json;

use crate::DaemonError;
use crate::paths::private_folder;
use crate::route::Router;
use crate::rpc;

/// The longest request line accepted.
pub const MAX_LINE: usize = 1 << 20;
/// The most connections served at once.
const MAX_CONNECTIONS: usize = 64;

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

/// Serves connections until the listener fails, each on its own thread.
pub fn serve(listener: &UnixListener, router: &Arc<Router>) {
    let open = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming().filter_map(Result::ok) {
        if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            open.fetch_sub(1, Ordering::SeqCst);
            continue;
        }
        let (router, served) = (Arc::clone(router), Arc::clone(&open));
        let spawned = thread::Builder::new()
            .name("connection".into())
            .spawn(move || {
                connection(&router, stream);
                served.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            open.fetch_sub(1, Ordering::SeqCst);
        }
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
                let _ = writeln!(writer, "{reply}");
                return;
            }
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        if text.trim().is_empty() {
            continue;
        }
        if let Some(reply) = rpc::handle(router, text.trim())
            && writeln!(writer, "{reply}").is_err()
        {
            return;
        }
    }
}
