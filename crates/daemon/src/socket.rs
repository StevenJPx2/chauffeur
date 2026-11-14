use std::io::{BufRead, BufReader, Read, Write};
use std::mem::DropGuard;
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use chauffeur_protocol::Rejection;
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
/// navigation does not hold up the next call. A call is checked and admitted
/// before its thread starts. `$/cancelRequest` cancels a call by its ID; a
/// notification for any other method runs nothing. The connection closing,
/// or a reply that cannot be written, cancels every call it carries. An
/// oversized line gets an error and closes the connection.
fn connection(router: &Router, stream: UnixStream) {
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    // A caller that stops reading cannot hold its calls' places for long.
    let _ = writer.set_write_timeout(Some(WRITE));
    let wire = Wire {
        writer: Mutex::new(writer),
        dead: AtomicBool::new(false),
    };
    let calls = Calls::default();
    thread::scope(|scope| {
        let mut reader = BufReader::new(stream);
        let mut line = Vec::new();
        while let Some(request) = next(&mut reader, &mut line, &wire) {
            if request.method == rpc::CANCEL {
                calls.cancel(request.params.get("id"));
                continue;
            }
            let Some(id) = request.id.clone() else {
                continue;
            };
            if let Err(refused) = calls.room(&id) {
                let _ = wire.send(&refused);
                continue;
            }
            let call = match rpc::prepare(router, request) {
                Ok(call) => call,
                Err(refused) => {
                    let _ = wire.send(&refused);
                    continue;
                }
            };
            let cancel = calls.open(id.clone());
            let (calls, wire) = (&calls, &wire);
            scope.spawn(move || {
                let notify = |message: Value| {
                    if wire.send(&message).is_err() {
                        calls.cancel_all();
                    }
                };
                let (reply, permit) = call.answer(router, &cancel, &notify);
                calls.close(&id);
                if wire.send(&reply).is_err() {
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
    wire: &Wire,
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
                let _ = wire.send(&reply);
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
            Err(reply) => wire.send(&reply).ok()?,
        }
    }
}

/// The writing side of a connection. Once a write fails, or times out
/// partway through a line, the connection is dead: nothing more is written,
/// and shutting it down ends the reader too.
struct Wire {
    writer: Mutex<UnixStream>,
    dead: AtomicBool,
}

impl Wire {
    /// Writes `message` and its newline in one write. Formatted straight to
    /// the socket, a message would go out in many small writes, each waking
    /// the caller.
    fn send(&self, message: &Value) -> std::io::Result<()> {
        if self.dead.load(Ordering::SeqCst) {
            return Err(std::io::ErrorKind::BrokenPipe.into());
        }
        let mut line = message.to_string();
        line.push('\n');
        let mut writer = self.writer.lock().unwrap_or_else(PoisonError::into_inner);
        let written = writer.write_all(line.as_bytes());
        if written.is_err() {
            self.dead.store(true, Ordering::SeqCst);
            let _ = writer.shutdown(Shutdown::Both);
        }
        written
    }
}

/// The calls open on one connection, each with its cancellation by ID.
#[derive(Default)]
struct Calls {
    open: Mutex<Vec<(Value, Cancel)>>,
}

impl Calls {
    /// Whether a call `id` may open: its ID is not in use, and fewer than
    /// [`MAX_CALLS`] are open. Only the reader opens calls, so the answer
    /// holds until it does.
    fn room(&self, id: &Value) -> Result<(), Value> {
        let (taken, count) = {
            let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
            (open.iter().any(|(open, _)| open == id), open.len())
        };
        if taken {
            return Err(rpc::duplicate(id));
        }
        if count >= MAX_CALLS {
            let busy = Rejection::Busy.to_error();
            return Err(json!({ "jsonrpc": "2.0", "id": id, "error": busy }));
        }
        Ok(())
    }

    /// Opens the call `id` and gives its cancellation.
    fn open(&self, id: Value) -> Cancel {
        let cancel = Cancel::new();
        self.open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push((id, cancel.clone()));
        cancel
    }

    /// Forgets the call `id` once it has its reply.
    fn close(&self, id: &Value) {
        let mut open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        open.retain(|(open, _)| open != id);
    }

    /// Cancels the open call with `id`.
    fn cancel(&self, id: Option<&Value>) {
        let open = self.open.lock().unwrap_or_else(PoisonError::into_inner);
        for (_, cancel) in open.iter().filter(|(open, _)| Some(open) == id) {
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
