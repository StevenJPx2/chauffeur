use serde_json::{Map, Value};

use crate::then::Then;

/// What a rule asks its caller to do, such as `withhold` an event, named by
/// the rule that produced it. On the wire it is one object:
/// `{ "rule": "sourcefed/triage", "withhold": "event" }`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Intent {
    rule: String,
    name: String,
    payload: Value,
}

impl Intent {
    /// The intent `then` returns for `rule`.
    #[must_use]
    pub fn new(rule: &str, then: &Then) -> Self {
        Self {
            rule: rule.to_owned(),
            name: then.name().to_owned(),
            payload: then.payload().clone(),
        }
    }

    /// Reads an intent from the wire: `rule` and exactly one other field.
    #[must_use]
    pub fn from_json(value: &Value) -> Option<Self> {
        let fields = value.as_object()?;
        let rule = fields.get("rule")?.as_str()?;
        let mut others = fields.iter().filter(|(key, _)| *key != "rule");
        let (name, payload) = others.next()?;
        if others.next().is_some() {
            return None;
        }
        Some(Self {
            rule: rule.to_owned(),
            name: name.clone(),
            payload: payload.clone(),
        })
    }

    /// The intent as it goes on the wire.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut fields = Map::new();
        fields.insert("rule".into(), Value::String(self.rule.clone()));
        fields.insert(self.name.clone(), self.payload.clone());
        Value::Object(fields)
    }

    /// The rule that produced it.
    #[must_use]
    pub fn rule(&self) -> &str {
        &self.rule
    }

    /// Its name, such as `withhold`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What it carries, such as `"event"`.
    #[must_use]
    pub const fn payload(&self) -> &Value {
        &self.payload
    }
}
