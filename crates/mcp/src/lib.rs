//! Thin MCP stdio adapter over the daemon client.

use chauffeur_core::{AgentContext, DaemonClient, SkillContext, Target};
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
            "name": "steer_idle",
            "description": "Evaluate an idle agent context and queue applicable reminders.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "object" },
                    "context": { "type": "object" }
                },
                "required": ["target", "context"]
            }
        },
        {
            "name": "reminders_read",
            "description": "Read queued reminders for a host target.",
            "inputSchema": target_schema()
        },
        {
            "name": "reminders_acknowledge",
            "description": "Acknowledge reminders delivered to an agent session.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": { "type": "object" },
                    "reminder_ids": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["target", "reminder_ids"]
            }
        },
        {
            "name": "skills_list",
            "description": "List validated skill contracts loaded by the daemon.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "skill_evaluate",
            "description": "Evaluate a validated skill contract against a host event and evidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "skill_id": { "type": "string" },
                    "context": { "type": "object" }
                },
                "required": ["skill_id", "context"]
            }
        }
    ] })
}

fn target_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "target": { "type": "object" } },
        "required": ["target"]
    })
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
        "steer_idle" => {
            let target = field::<Target>(&arguments, "target")?;
            let context = field::<AgentContext>(&arguments, "context")?;
            serde_json::to_value(client.steer(target, context).await?)
                .map_err(|error| error.to_string())?
        }
        "reminders_read" => {
            let target = field::<Target>(&arguments, "target")?;
            serde_json::to_value(client.reminders(target).await?)
                .map_err(|error| error.to_string())?
        }
        "reminders_acknowledge" => {
            let target = field::<Target>(&arguments, "target")?;
            let ids = field::<Vec<String>>(&arguments, "reminder_ids")?;
            client.acknowledge(target, ids).await?;
            json!({ "ok": true })
        }
        "skills_list" => {
            serde_json::to_value(client.skills().await?).map_err(|error| error.to_string())?
        }
        "skill_evaluate" => {
            let skill_id = field::<String>(&arguments, "skill_id")?;
            let context = field::<SkillContext>(&arguments, "context")?;

            serde_json::to_value(client.evaluate_skill(skill_id, context).await?)
                .map_err(|error| error.to_string())?
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
