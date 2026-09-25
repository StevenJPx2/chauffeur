//! The rule format: a trigger, a gate of exact facts, one or two ordered Jev
//! judgments, and the context delivered once every judgment holds.

use std::collections::HashSet;

use chauffeur_core::Delivery;
use serde::Deserialize;

pub const SCHEMA_VERSION: u8 = 2;
const MAX_STEPS: usize = 2;
const MAX_ID_BYTES: usize = 64;
const MAX_NAME_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 1_024;
const MAX_LIST: usize = 16;
const MAX_COOLDOWN_SECS: u64 = 86_400;

/// One rule. Facts admit it, Jev confirms its steps in order, and its
/// context is delivered once the last step holds.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub schema_version: u8,
    pub id: String,
    pub name: String,
    /// The signal that can fire the rule.
    pub on: Trigger,
    #[serde(default)]
    pub when: Gate,
    /// One or two judgments; the second is asked only after the first holds.
    pub steps: Vec<Step>,
    pub then: Then,
    /// Higher first when more rules hold than one signal delivers.
    #[serde(default)]
    pub priority: u8,
    /// Deliver at most once per agent within the gate's history window.
    #[serde(default)]
    pub once: bool,
    #[serde(default)]
    pub cooldown_seconds: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// After each tool call; the rule can ask about that call.
    ToolResult,
    /// When the agent's turn ends.
    TurnEnd,
}

/// How far back the gate's facts reach, and how long `once` lasts.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum History {
    /// Since the latest user message, in the current workspace.
    #[default]
    Turn,
    /// The whole session, in any workspace.
    Session,
}

/// Exact preconditions checked before Jev is asked. Structural facts live
/// here because the model cannot see them reliably: it scored "the agent
/// opened a pull request" at 0.24 on a state that contained
/// `github_open_pr`. Empty lists mean no constraint.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Gate {
    /// For `tool_result`: the call's tool must be one of these.
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub history: History,
    /// Every one of these tools was called.
    #[serde(default)]
    pub tools_called: Vec<String>,
    /// At least one of these tools was called.
    #[serde(default)]
    pub tools_called_any: Vec<String>,
    /// None of these tools was called.
    #[serde(default)]
    pub tools_not_called: Vec<String>,
    /// The agent's status is one of these: `implementing` or `in_review`.
    #[serde(default)]
    pub status: Vec<String>,
    /// The agent's source is one of these: `jira` or `github`.
    #[serde(default)]
    pub source: Vec<String>,
    /// At least one of these integration events arrived, as `source:kind`.
    #[serde(default)]
    pub hooks: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub question: String,
    /// Confirmed when P(yes) is at least this…
    pub yes_at_or_above: f32,
    /// …and the answer is at least this confident.
    pub minimum_confidence: f32,
}

/// The context a confirmed rule delivers.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Then {
    /// `steer` for a tool result; `resume` or `wait` at a turn end.
    pub delivery: Delivery,
    pub text: String,
    /// Shown with the context; the rule's name when absent.
    #[serde(default)]
    pub label: Option<String>,
    /// A skill handed over with the text, such as `slack-cli`.
    #[serde(default)]
    pub skill: Option<String>,
}

impl Rule {
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let rule: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;

        rule.validate()?;

        Ok(rule)
    }

    /// The label its context is delivered with.
    #[must_use]
    pub fn label(&self) -> &str {
        self.then.label.as_deref().unwrap_or(&self.name)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported schema_version {} (rules use {SCHEMA_VERSION})",
                self.schema_version
            ));
        }
        if !identifier(&self.id) {
            return Err(format!(
                "id {:?} must be 1-{MAX_ID_BYTES} of [a-z0-9_-]",
                self.id
            ));
        }
        if !bounded(&self.name, MAX_NAME_BYTES) {
            return Err(format!(
                "{}: name must be 1-{MAX_NAME_BYTES} bytes",
                self.id
            ));
        }
        if self.cooldown_seconds > MAX_COOLDOWN_SECS {
            return Err(format!(
                "{}: cooldown_seconds exceeds {MAX_COOLDOWN_SECS}",
                self.id
            ));
        }

        self.validate_trigger()?;
        self.validate_gate()?;
        self.validate_steps()?;
        self.validate_then()
    }

    /// A tool-result rule watches named tools and steers the running turn;
    /// a turn-end rule resumes the agent or waits for its next turn.
    fn validate_trigger(&self) -> Result<(), String> {
        let (watched, deliveries): (bool, &[Delivery]) = match self.on {
            Trigger::ToolResult => (true, &[Delivery::Steer]),
            Trigger::TurnEnd => (false, &[Delivery::Resume, Delivery::Wait]),
        };

        if watched && self.when.tools.is_empty() {
            return Err(format!(
                "{}: a tool_result rule must list when.tools",
                self.id
            ));
        }
        if !watched && !self.when.tools.is_empty() {
            return Err(format!(
                "{}: when.tools applies only to tool_result",
                self.id
            ));
        }
        if !deliveries.contains(&self.then.delivery) {
            return Err(format!(
                "{}: {:?} delivery does not fit a {:?} rule",
                self.id, self.then.delivery, self.on
            ));
        }

        Ok(())
    }

    fn validate_gate(&self) -> Result<(), String> {
        let gate = &self.when;
        let lists = [
            ("tools", &gate.tools),
            ("tools_called", &gate.tools_called),
            ("tools_called_any", &gate.tools_called_any),
            ("tools_not_called", &gate.tools_not_called),
            ("status", &gate.status),
            ("source", &gate.source),
            ("hooks", &gate.hooks),
        ];

        for (field, values) in lists {
            if values.len() > MAX_LIST {
                return Err(format!("{}: when.{field} lists over {MAX_LIST}", self.id));
            }
            if let Some(value) = values.iter().find(|value| !fact(value)) {
                return Err(format!("{}: invalid when.{field} value {value:?}", self.id));
            }
        }

        Ok(())
    }

    fn validate_steps(&self) -> Result<(), String> {
        if !(1..=MAX_STEPS).contains(&self.steps.len()) {
            return Err(format!("{}: steps must list 1-{MAX_STEPS}", self.id));
        }

        let mut ids = HashSet::new();

        for step in &self.steps {
            if !identifier(&step.id) || !ids.insert(&step.id) {
                return Err(format!(
                    "{}: step id {:?} is invalid or repeated",
                    self.id, step.id
                ));
            }
            if !bounded(&step.question, MAX_TEXT_BYTES) {
                return Err(format!(
                    "{}: step {} question must be 1-{MAX_TEXT_BYTES} bytes",
                    self.id, step.id
                ));
            }
            // Below an even split, "yes" would fire on a likely no.
            if !(0.5..=1.0).contains(&step.yes_at_or_above)
                || !(0.0..=1.0).contains(&step.minimum_confidence)
            {
                return Err(format!(
                    "{}: step {} needs yes_at_or_above in 0.5-1 and minimum_confidence in 0-1",
                    self.id, step.id
                ));
            }
        }

        Ok(())
    }

    fn validate_then(&self) -> Result<(), String> {
        let then = &self.then;

        if !bounded(&then.text, MAX_TEXT_BYTES) {
            return Err(format!(
                "{}: then.text must be 1-{MAX_TEXT_BYTES} bytes",
                self.id
            ));
        }
        if then
            .label
            .as_ref()
            .is_some_and(|label| !bounded(label, MAX_NAME_BYTES))
        {
            return Err(format!(
                "{}: then.label must be 1-{MAX_NAME_BYTES} bytes",
                self.id
            ));
        }
        if then.skill.as_ref().is_some_and(|skill| !identifier(skill)) {
            return Err(format!("{}: then.skill must be a skill name", self.id));
        }

        Ok(())
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

/// A tool name, status, source, or `source:kind` hook.
fn fact(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'"')
}

fn bounded(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max_bytes
}
