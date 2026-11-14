use std::io::{BufRead, BufReader, Read, Write};
use std::mem::DropGuard;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use chauffeur_runtime::Cancel;
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
/// How long one write to a caller may take before the connection is given up.
const WRITE: Duration = Duration::from_secs(5);
/// The most calls open at once on one connection.
pub const MAX_CALLS: usize = 16;
/// The code for a call the daemon cannot take now.
const BUSY: i64 = -32_000;

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
/// thread; a connection that fails to be accepted is skipped. A connection
/// past [`MAX_CONNECTIONS`] is told the daemon is busy and closed.
pub fn serve(listener: &UnixListener, router: &Arc<Router>) -> ! {
    let open = Arc::new(AtomicUsize::new(0));
    loop {
        let Ok((stream, _)) = listener.accept() else {
            continue;
        };
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

/// Tells a caller past the limit why it is closed, so it does not take the
/// closed connection for a daemon that stopped. A caller that is not reading
/// is not waited for.
fn busy(mut stream: UnixStream) {
    let reply = json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32_000,
        "message": format!("the daemon is busy: it serves {MAX_CONNECTIONS} connections already") } });
    if stream.set_write_timeout(Some(BUSY_WRITE)).is_ok() {
        let mut line = reply.to_string();
        line.push('\n');
        let _ = stream.write_all(line.as_bytes());
    }
}

/// Reads request lines and answers each on its own thread, so a slow
/// navigation does not hold up the next call. `$/cancelRequest` cancels a
/// call by its ID, and the connection closing, or a reply that cannot be
/// written, cancels every call it carries. An oversized line gets an error
/// and closes the connection.
fn connection(router: &Router, stream: UnixStream) {
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    // A caller that stops reading cannot hold its calls' places for long.
    let _ = writer.set_write_timeout(Some(WRITE));
    let writer = Mutex::new(writer);
    let calls = Calls::default();
    thread::scope(|scope| {
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        while let Some(request) = next(&mut reader, &mut line, &writer) {
            if request.method == rpc::CANCEL {
                calls.cancel(request.params.get("id"));
                continue;
            }
            let Some(cancel) = calls.open(request.id.as_ref()) else {
                let id = request.id.unwrap_or(Value::Null);
                let busy = rpc::error(
                    &id,
                    BUSY,
                    &format!("{MAX_CALLS} calls are open on this connection"),
                );
                if send(&writer, &busy).is_err() {
                    break;
                }
                continue;
            };
            let (calls, writer) = (&calls, &writer);
            scope.spawn(move || {
                let notify = |message: Value| {
                    if send(writer, &message).is_err() {
                        calls.cancel_all();
                    }
                };
                let id = request.id.clone();
                let (reply, permit) = rpc::answer(router, request, &cancel, &notify);
                calls.close(id.as_ref());
                if let Some(reply) = reply
                    && send(writer, &reply).is_err()
                {
                    calls.cancel_all();
                }
                // Held until the reply is written.
                drop(permit);
            });
        }
        // The caller is gone: nothing it asked for is worth finishing.
        calls.cancel_all();
    });
}

/// The next request on the connection, answering any line that is not one.
/// None once the connection closes, a line is over [`MAX_LINE`], or an
/// error cannot be written.
fn next(
    reader: &mut BufReader<UnixStream>,
    line: &mut Vec<u8>,
    writer: &Mutex<UnixStream>,
) -> Option<rpc::Request> {
    loop {
        line.clear();
        let limit = u64::try_from(MAX_LINE)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        match reader.by_ref().take(limit).read_until(b'\n', line) {
            Ok(0) | Err(_) => return None,
            Ok(_) if line.len() > MAX_LINE => {
                let reply = rpc::error(&Value::Null, -32_600, "the request line is over 1 MiB");
                let _ = send(writer, &reply);
                return None;
            }
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(line);
        if text.trim().is_empty() {
            continue;
        }
        match rpc::parse(text.trim()) {
            Ok(request) => return Some(request),
            Err(reply) => send(writer, &reply).ok()?,
        }
    }
}

/// The calls open on one connection, each with its cancellation by ID.
#[derive(Default)]
struct Calls {
    open: Mutex<Vec<(Option<Value>, Cancel)>>,
}

impl Calls {
    /// A cancellation for a new call, unless [`MAX_CALLS`] are open.
    fn open(&self, id: Option<&Value>) -> Option<Cancel> {
        let cancel = Cancel::new();
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if open.len() >= MAX_CALLS {
            return None;
        }
        open.push((id.cloned(), cancel.clone()));
        drop(open);
        Some(cancel)
    }

    /// Forgets the call `id` once it has its reply.
    fn close(&self, id: Option<&Value>) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(index) = open.iter().position(|(open, _)| open.as_ref() == id) {
            open.swap_remove(index);
        }
    }

    /// Cancels every open call with `id`.
    fn cancel(&self, id: Option<&Value>) {
        let Some(id) = id else {
            return;
        };
        let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, cancel) in open.iter().filter(|(open, _)| open.as_ref() == Some(id)) {
            cancel.cancel();
        }
    }

    /// Cancels every open call.
    fn cancel_all(&self) {
        let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, cancel) in open.iter() {
            cancel.cancel();
        }
    }
}

/// Writes `reply` and its newline in one write. Formatted straight to the
/// socket, a reply would go out in many small writes, each waking the
/// caller.
fn send(writer: &Mutex<UnixStream>, reply: &Value) -> std::io::Result<()> {
    let mut line = reply.to_string();
    line.push('\n');
    writer
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .write_all(line.as_bytes())
}
