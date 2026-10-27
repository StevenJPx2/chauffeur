use serde::Deserialize;
use serde_json::Value;

use crate::event::Event;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputFilter {
    contains: String,
}

/// A rule's cheap filter on its event, checked before any judge is asked.
/// An empty filter matches every event.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// The event's `tool` must be one of these.
    #[serde(default)]
    tools: Vec<String>,
    /// Some text in the event's `input` must contain this.
    input: Option<InputFilter>,
}

impl When {
    /// Whether `event` passes the filter.
    #[must_use]
    pub fn matches(&self, event: &Event) -> bool {
        let tool = event.field("tool").and_then(Value::as_str);
        let tools = self.tools.is_empty()
            || tool.is_some_and(|tool| self.tools.iter().any(|listed| listed == tool));
        let input = self.input.as_ref().is_none_or(|filter| {
            event
                .field("input")
                .is_some_and(|input| contains(input, &filter.contains))
        });
        tools && input
    }
}

/// Whether any string within `value` contains `needle`.
fn contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(items) => items.iter().any(|item| contains(item, needle)),
        Value::Object(fields) => fields.values().any(|field| contains(field, needle)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}
