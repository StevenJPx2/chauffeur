use std::time::Duration;

use chauffeur_protocol::Event;
use serde_json::{Value, json};

use crate::route::Router;

/// The longest a caller may give rules to answer.
const MAX_DEADLINE: Duration = Duration::from_secs(30);
const DEFAULT_DEADLINE: Duration = Duration::from_secs(3);

const PARSE_ERROR: i64 = -32_700;
const INVALID_REQUEST: i64 = -32_600;
const METHOD_NOT_FOUND: i64 = -32_601;
const INVALID_PARAMS: i64 = -32_602;

/// Answers one JSON-RPC 2.0 request line. A notification, which has no
/// `id`, gets no reply.
pub fn handle(router: &Router, line: &str) -> Option<Value> {
    let request: Value = match serde_json::from_str(line) {
        Ok(request) => request,
        Err(_) => return Some(error(&Value::Null, PARSE_ERROR, "not JSON")),
    };
    let id = request.get("id").cloned();
    let text = |field: &str| request.get(field).and_then(Value::as_str);
    let (Some("2.0"), Some(method)) = (text("jsonrpc"), text("method")) else {
        return Some(error(
            &id.unwrap_or(Value::Null),
            INVALID_REQUEST,
            "not a JSON-RPC 2.0 request",
        ));
    };
    let result = call(
        router,
        method,
        request.get("params").unwrap_or(&Value::Null),
    );
    let id = id?;
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => error(&id, code, &message),
    })
}

fn call(router: &Router, method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "chauffeur.emit" => {
            let event = params
                .get("event")
                .ok_or_else(|| (INVALID_PARAMS, "no `event`".to_owned()))?;
            let event = Event::parse(&event.to_string())
                .map_err(|error| (INVALID_PARAMS, error.to_string()))?;
            let deadline = params
                .get("deadline_ms")
                .and_then(Value::as_u64)
                .map_or(DEFAULT_DEADLINE, Duration::from_millis)
                .min(MAX_DEADLINE);
            Ok(json!({ "actions": router.emit(&event, deadline) }))
        }
        "chauffeur.events" => Ok(router.events()),
        "chauffeur.status" => {
            let loaded = router.loaded();
            let rules: Vec<&str> = loaded
                .iter()
                .flat_map(|loaded| loaded.rules.iter().map(|rule| rule.id()))
                .collect();
            let refused = loaded
                .as_ref()
                .map(|loaded| loaded.refused.clone())
                .unwrap_or_default();
            Ok(
                json!({ "version": env!("CARGO_PKG_VERSION"), "protocol": 1, "rules": rules, "refused": refused }),
            )
        }
        other => Err((METHOD_NOT_FOUND, format!("no method `{other}`"))),
    }
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}
