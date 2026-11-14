use std::time::Duration;

use chauffeur_protocol::{Event, Navigate, Rejection};
use chauffeur_runtime::{Cancel, Permit, Source};
use serde_json::{Value, json};

use crate::route::Router;

/// The longest a caller may give rules to answer.
const MAX_DEADLINE: Duration = Duration::from_secs(30);
const DEFAULT_DEADLINE: Duration = Duration::from_secs(3);

const PARSE_ERROR: i64 = -32_700;
const INVALID_REQUEST: i64 = -32_600;
const METHOD_NOT_FOUND: i64 = -32_601;
const INVALID_PARAMS: i64 = -32_602;

/// The notification that cancels a request on the same connection.
pub const CANCEL: &str = "$/cancelRequest";

/// One JSON-RPC 2.0 request or notification, read from a line.
pub struct Request {
    /// Its ID; a notification has none and gets no reply.
    pub id: Option<Value>,
    /// The method it calls.
    pub method: String,
    /// Its params, or null.
    pub params: Value,
}

/// A request read from `line`, or the error reply for a line that is not one.
pub fn parse(line: &str) -> Result<Request, Value> {
    let request: Value =
        serde_json::from_str(line).map_err(|_| error(&Value::Null, PARSE_ERROR, "not JSON"))?;
    let id = request.get("id").cloned();
    let text = |field: &str| request.get(field).and_then(Value::as_str);
    let (Some("2.0"), Some(method)) = (text("jsonrpc"), text("method")) else {
        return Err(error(
            &id.unwrap_or(Value::Null),
            INVALID_REQUEST,
            "not a JSON-RPC 2.0 request",
        ));
    };
    Ok(Request {
        method: method.to_owned(),
        params: request.get("params").cloned().unwrap_or(Value::Null),
        id,
    })
}

/// A call checked and admitted, ready to run on a thread of its own.
pub struct Call {
    id: Value,
    work: Work,
    /// Held until the reply is written; none for a call that needs no place.
    permit: Option<Permit>,
}

enum Work {
    Emit(Event, Duration),
    Navigate(Navigate),
    Status,
}

/// Checks `request`'s params, then admits it for its source, so a call that
/// is malformed, over a limit or busy is answered at once, before any thread
/// starts.
///
/// # Errors
///
/// The error reply to send.
pub fn prepare(router: &Router, request: Request) -> Result<Call, Value> {
    let id = request.id.unwrap_or(Value::Null);
    let refuse = |error: Value| json!({ "jsonrpc": "2.0", "id": id, "error": error });
    let (work, source) = match request.method.as_str() {
        "chauffeur.emit" => (
            Work::Emit(
                event(&request.params).map_err(refuse)?,
                deadline(&request.params),
            ),
            Some(Source::Harness),
        ),
        "chauffeur.navigate" => {
            let navigate = Navigate::parse(&request.params)
                .map_err(|rejection| refuse(rejection.to_error()))?;
            (Work::Navigate(navigate), Some(Source::Integration))
        }
        "chauffeur.status" => (Work::Status, None),
        other => {
            return Err(refuse(error_object(
                METHOD_NOT_FOUND,
                &format!("no method `{other}`"),
            )));
        }
    };
    let permit = source
        .map(|source| router.admit(source))
        .transpose()
        .map_err(|_| refuse(Rejection::Busy.to_error()))?;
    Ok(Call { id, work, permit })
}

impl Call {
    /// Runs the call, with `notify` given each notification it streams, and
    /// returns its reply and the permit to drop once the reply is written.
    pub fn answer(
        self,
        router: &Router,
        cancel: &Cancel,
        notify: &(dyn Fn(Value) + Sync),
    ) -> (Value, Option<Permit>) {
        let id = self.id;
        let result = match self.work {
            Work::Emit(event, deadline) => {
                Ok(json!({ "actions": router.emit(&event, deadline, cancel) }))
            }
            Work::Navigate(request) => {
                let progress = |mut event: Value| {
                    if let Some(fields) = event.as_object_mut() {
                        fields.insert("id".into(), id.clone());
                    }
                    notify(
                        json!({ "jsonrpc": "2.0", "method": "chauffeur.event", "params": event }),
                    );
                };
                router
                    .navigate(request, cancel, &progress)
                    .map_err(|rejection| rejection.to_error())
            }
            Work::Status => Ok(status(router)),
        };
        let reply = match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        };
        (reply, self.permit)
    }
}

fn event(params: &Value) -> Result<Event, Value> {
    let event = params
        .get("event")
        .ok_or_else(|| error_object(INVALID_PARAMS, "no `event`"))?;
    Event::parse(&event.to_string())
        .map_err(|error| error_object(INVALID_PARAMS, &error.to_string()))
}

fn deadline(params: &Value) -> Duration {
    params
        .get("deadline_ms")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_DEADLINE, Duration::from_millis)
        .min(MAX_DEADLINE)
}

fn status(router: &Router) -> Value {
    let loaded = router.loaded();
    let rules: Vec<&str> = loaded
        .iter()
        .flat_map(|loaded| loaded.rules.iter().map(|rule| rule.id()))
        .collect();
    let refused = loaded
        .as_ref()
        .map_or_default(|loaded| loaded.refused.clone());
    json!({ "version": env!("CARGO_PKG_VERSION"), "protocol": 1, "rules": rules, "refused": refused })
}

fn error_object(code: i64, message: &str) -> Value {
    json!({ "code": code, "message": message })
}

/// An error reply to `id`.
pub fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": error_object(code, message) })
}

/// The error reply to a call whose ID is already open on its connection.
pub fn duplicate(id: &Value) -> Value {
    error(
        id,
        INVALID_REQUEST,
        "a call with this id is already open on this connection",
    )
}
