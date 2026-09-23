//! Thin MCP stdio adapter over the daemon client.

use chauffeur_core::{DaemonClient, Signal};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

pub async fn run_stdio(client: DaemonClient) -> Result<(), String> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut output = tokio::io::stdout();

    while let Some(line) = lines.next_line().await.map_err(|error| error.to_string())? {
        let request: Value =
            serde_json::from_str(&line).map_err(|error| format!("invalid JSON-RPC: {error}"))?;
        let Some(response) = handle(&client, request).await else {
            continue;
        };
        let mut encoded = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
        encoded.push(b'\n');
        output
            .write_all(&encoded)
            .await
            .map_err(|error| error.to_string())?;
        output.flush().await.map_err(|error| error.to_string())?;
    }

    Ok(())
}

async fn handle(client: &DaemonClient, request: Value) -> Option<Value> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(initialize()),
        "notifications/initialized" => return None,
        "tools/list" => Ok(tool_list()),
        "tools/call" => call_tool(client, params).await,
        "ping" => Ok(json!({})),
        _ => Err(format!("unsupported MCP method {method}")),
    };

    match result {
        Ok(result) => Some(json!({ "jsonrpc": "2.0", "id": id, "result": result })),
        Err(message) => Some(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32602, "message": message }
        })),
    }
}

fn initialize() -> Value {
    json!({
        "protocolVersion": "2025-11-25",
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "chauffeur", "version": env!("CARGO_PKG_VERSION") }
    })
}

fn tool_list() -> Value {
    json!({ "tools": [
        {
            "name": "signal",
            "description": "Report one agent observation to the Chauffeur engine and return its effects.",
            "inputSchema": {
                "type": "object",
                "properties": { "signal": { "type": "object" } },
                "required": ["signal"]
            }
        }
    ] })
}

async fn call_tool(client: &DaemonClient, params: Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("tool call needs a name")?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let value = match name {
        "signal" => {
            let signal = field::<Signal>(&arguments, "signal")?;

            serde_json::to_value(client.signal(signal).await?).map_err(|error| error.to_string())?
        }
        _ => return Err(format!("unknown tool {name}")),
    };

    Ok(json!({ "content": [{ "type": "text", "text": value.to_string() }] }))
}

fn field<T: serde::de::DeserializeOwned>(value: &Value, name: &str) -> Result<T, String> {
    let field = value
        .get(name)
        .cloned()
        .ok_or_else(|| format!("missing {name}"))?;
    serde_json::from_value(field).map_err(|error| format!("invalid {name}: {error}"))
}
