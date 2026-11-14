//! `chauffeur navigate`: one request on stdin, the JSON-RPC messages that
//! come back on stdout, one per line, and an exit status a script can test.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// How much longer than the request's deadline the daemon is waited for.
const MARGIN: Duration = Duration::from_secs(2);

/// The deadline the daemon gives a request that names none.
const DEFAULT_DEADLINE_MS: u64 = 3_000;

/// The ID of the one request sent.
const ID: u64 = 1;

/// How a navigation ended, as the command's exit status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended {
    /// Intents or a verdict came back: 0.
    Answered,
    /// The daemon rejected the request: 1.
    Rejected,
    /// No daemon answered, or it speaks another protocol: 2.
    Unavailable,
}

impl From<Ended> for ExitCode {
    fn from(ended: Ended) -> Self {
        match ended {
            Ended::Answered => Self::SUCCESS,
            Ended::Rejected => Self::from(1),
            Ended::Unavailable => Self::from(2),
        }
    }
}

/// Sends the request read from `input` to the daemon on `socket` and
/// writes each message it sends back to `output`, until the reply.
pub fn navigate(socket: &Path, input: &mut impl Read, output: &mut impl Write) -> Ended {
    let mut text = String::new();
    let params: Value = match input
        .read_to_string(&mut text)
        .map(|_| serde_json::from_str(&text))
    {
        Ok(Ok(params)) => params,
        Ok(Err(error)) => {
            return complain(
                &format!("the request on stdin is not JSON: {error}"),
                Ended::Rejected,
            );
        }
        Err(error) => return complain(&format!("cannot read stdin: {error}"), Ended::Rejected),
    };
    let Ok(mut stream) = UnixStream::connect(socket) else {
        return complain(
            &format!("no daemon answers on {}", socket.display()),
            Ended::Unavailable,
        );
    };
    let deadline = params
        .get("deadline_ms")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_DEADLINE_MS);
    let wait = Duration::from_millis(deadline).saturating_add(MARGIN);
    let until = Instant::now().checked_add(wait);
    let request =
        json!({ "jsonrpc": "2.0", "id": ID, "method": "chauffeur.navigate", "params": params });
    if writeln!(stream, "{request}").is_err() {
        return complain("the daemon closed the connection", Ended::Unavailable);
    }
    let Ok(reader) = stream.try_clone() else {
        return complain("cannot read from the daemon", Ended::Unavailable);
    };
    relay(&stream, BufReader::new(reader), until, output)
}

/// Copies each message to `output` until the reply to the request, or
/// `until`, however many events arrive before it.
fn relay(
    stream: &UnixStream,
    mut reader: impl BufRead,
    until: Option<Instant>,
    output: &mut impl Write,
) -> Ended {
    let mut line = String::new();
    loop {
        let left = until.map(|until| until.saturating_duration_since(Instant::now()));
        // A zero timeout is refused, which is how a passed deadline shows.
        if stream.set_read_timeout(left).is_err() {
            return complain("the daemon did not answer in time", Ended::Unavailable);
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => return complain("the daemon did not answer in time", Ended::Unavailable),
        }
        let text = line.trim_end();
        // A reader that went away, such as `head -1`, has what it wanted.
        if writeln!(output, "{text}").is_err() {
            return Ended::Rejected;
        }
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        if message.get("id") != Some(&json!(ID)) {
            continue;
        }
        let reason = message
            .pointer("/error/data/reason")
            .and_then(Value::as_str);
        return match (message.get("result"), reason) {
            (Some(_), _) => Ended::Answered,
            (None, Some("unsupported_protocol")) => Ended::Unavailable,
            (None, _) => Ended::Rejected,
        };
    }
    complain(
        "the daemon closed the connection without answering",
        Ended::Unavailable,
    )
}

fn complain(message: &str, ended: Ended) -> Ended {
    eprintln!("chauffeur navigate: {message}");
    ended
}
