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

/// Answers `request`, with `notify` given each notification it streams.
/// The reply is none for a notification. The permit, when the call was
/// admitted, is held until the caller has sent the reply.
pub fn answer(
    router: &Router,
    request: Request,
    cancel: &Cancel,
    notify: &(dyn Fn(Value) + Sync),
) -> (Option<Value>, Option<Permit>) {
    let id = request.id.clone().unwrap_or(Value::Null);
    let (result, permit) = match request.method.as_str() {
        "chauffeur.emit" => emit(router, &request.params, cancel),
        "chauffeur.navigate" => navigate(router, &id, &request.params, cancel, notify),
        "chauffeur.status" => (Ok(status(router)), None),
        other => (
            Err(error_object(
                METHOD_NOT_FOUND,
                &format!("no method `{other}`"),
            )),
            None,
        ),
    };
    let reply = request.id.map(|id| match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    });
    (reply, permit)
}

fn emit(
    router: &Router,
    params: &Value,
    cancel: &Cancel,
) -> (Result<Value, Value>, Option<Permit>) {
    let event = params
        .get("event")
        .ok_or_else(|| error_object(INVALID_PARAMS, "no `event`"))
        .and_then(|event| {
            Event::parse(&event.to_string())
                .map_err(|error| error_object(INVALID_PARAMS, &error.to_string()))
        });
    let event = match event {
        Ok(event) => event,
        Err(error) => return (Err(error), None),
    };
    let Ok(permit) = router.admit(Source::Harness) else {
        return (Err(Rejection::Busy.to_error()), None);
    };
    let deadline = params
        .get("deadline_ms")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_DEADLINE, Duration::from_millis)
        .min(MAX_DEADLINE);
    let actions = router.emit(&event, deadline, cancel);
    (Ok(json!({ "actions": actions })), Some(permit))
}

fn navigate(
    router: &Router,
    id: &Value,
    params: &Value,
    cancel: &Cancel,
    notify: &(dyn Fn(Value) + Sync),
) -> (Result<Value, Value>, Option<Permit>) {
    let request = match Navigate::parse(params) {
        Ok(request) => request,
        Err(rejection) => return (Err(rejection.to_error()), None),
    };
    let Ok(permit) = router.admit(Source::Integration) else {
        return (Err(Rejection::Busy.to_error()), None);
    };
    let progress = |mut event: Value| {
        if let Some(fields) = event.as_object_mut() {
            fields.insert("id".into(), id.clone());
        }
        notify(json!({ "jsonrpc": "2.0", "method": "chauffeur.event", "params": event }));
    };
    let result = router
        .navigate(request, cancel, &progress)
        .map_err(|rejection| rejection.to_error());
    (result, Some(permit))
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
