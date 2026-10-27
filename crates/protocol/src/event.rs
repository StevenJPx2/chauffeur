use std::fmt::Write as _;

use serde_json::{Map, Value};

/// Why an event was refused.
#[derive(Debug, thiserror::Error)]
pub enum EventError {
    /// Not a JSON object.
    #[error("an event is a JSON object: {0}")]
    Json(#[from] serde_json::Error),
    /// The object has no string `event` naming it.
    #[error("an event needs its name in `event`")]
    Unnamed,
}

/// A standard harness event, such as `tool.after`: its name and its fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event {
    name: String,
    fields: Map<String, Value>,
}

impl Event {
    /// An event from its JSON object, named by its `event` field.
    ///
    /// # Errors
    ///
    /// Not a JSON object, or no string `event` field.
    pub fn parse(text: &str) -> Result<Self, EventError> {
        let Value::Object(mut fields) = serde_json::from_str(text)? else {
            return Err(EventError::Unnamed);
        };
        match fields.remove("event") {
            Some(Value::String(name)) => Ok(Self { name, fields }),
            _ => Err(EventError::Unnamed),
        }
    }

    /// The event's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The field named `field`.
    #[must_use]
    pub fn field(&self, field: &str) -> Option<&Value> {
        self.fields.get(field)
    }

    /// The session the event belongs to, if it names one.
    #[must_use]
    pub fn session(&self) -> Option<&str> {
        self.field("session").and_then(Value::as_str)
    }

    /// The evidence blocks a rule on this event can name: `tool_call` (the
    /// tool and its input) and `tool_output` for tool events, and every other
    /// string field under its own name.
    #[must_use]
    pub fn evidence(&self) -> Vec<(String, String)> {
        let mut blocks = Vec::new();
        if let Some(tool) = self.field("tool").and_then(Value::as_str) {
            let mut call = format!("tool: {tool}\n");
            if let Some(Value::Object(input)) = self.field("input") {
                for (key, value) in input {
                    let _ = writeln!(call, "{key}: {}", text(value));
                }
            }
            blocks.push(("tool_call".to_owned(), call));
        }
        if let Some(output) = self.field("output") {
            blocks.push(("tool_output".to_owned(), text(output)));
        }
        for (name, value) in &self.fields {
            if let (false, Some(value)) = (
                matches!(name.as_str(), "tool" | "input" | "output"),
                value.as_str(),
            ) {
                blocks.push((name.clone(), value.to_owned()));
            }
        }
        blocks
    }
}

/// A string as itself, anything else as compact JSON.
fn text(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}
