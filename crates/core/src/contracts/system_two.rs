//! Provider-neutral System Two interface: a reasoning request with bounded
//! evidence and an output schema in, a verdict that fits the schema out.

use std::collections::HashSet;
use std::fmt;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};

/// Bytes of instructions and evidence one request may send.
pub const MAX_REASONING_INPUT_BYTES: usize = 32 * 1024;
/// Bytes a verdict may hold, serialized.
pub const MAX_VERDICT_BYTES: usize = 64 * 1024;
/// The evidence ID the engine gives the agent's recent activity.
pub const ACTIVITY: &str = "activity";

const MAX_SCHEMA_DEPTH: usize = 4;
const MAX_FIELDS: usize = 16;
const MAX_ITEMS: usize = 32;
const MAX_STRING_CHARS: usize = 2_048;
const MAX_ENUM_VALUES: usize = 32;

/// One piece of evidence, named so a verdict can cite it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Evidence {
    pub id: String,
    pub text: String,
}

/// A question for an LLM that needs reasoning over evidence.
#[derive(Clone, Debug, PartialEq)]
pub struct Reasoning {
    pub id: String,
    /// The execution profile that answers it, such as `completion-review`.
    pub profile: String,
    pub instructions: String,
    pub evidence: Vec<Evidence>,
    /// Whether the engine adds the agent's recent activity as evidence
    /// [`ACTIVITY`].
    pub activity: bool,
    pub schema: Schema,
}

impl Reasoning {
    /// Bytes of instructions and evidence this request sends.
    #[must_use]
    pub fn input_bytes(&self) -> usize {
        self.instructions.len()
            + self
                .evidence
                .iter()
                .map(|evidence| evidence.id.len() + evidence.text.len())
                .sum::<usize>()
    }

    fn evidence_ids(&self) -> HashSet<&str> {
        self.evidence
            .iter()
            .map(|evidence| evidence.id.as_str())
            .collect()
    }
}

/// The verdict shape: a subset of JSON Schema every provider can enforce.
/// Objects require every field and allow no others.
#[derive(Clone, Debug, PartialEq)]
pub enum Schema {
    Object(Vec<(String, Schema)>),
    Array {
        items: Box<Schema>,
        max_items: usize,
    },
    String {
        max_chars: usize,
    },
    Enum(Vec<String>),
    Boolean,
    Number {
        min: f64,
        max: f64,
    },
    /// The ID of one piece of the request's evidence.
    EvidenceId,
}

impl Schema {
    /// Whether the schema is within bounds.
    ///
    /// # Errors
    ///
    /// Names the first part out of bounds.
    pub fn check(&self) -> Result<(), String> {
        self.check_at(0)
    }

    fn check_at(&self, depth: usize) -> Result<(), String> {
        if depth > MAX_SCHEMA_DEPTH {
            return Err(format!("nested deeper than {MAX_SCHEMA_DEPTH}"));
        }

        match self {
            Self::Object(fields) => {
                if fields.is_empty() || fields.len() > MAX_FIELDS {
                    return Err(format!("an object needs 1-{MAX_FIELDS} fields"));
                }

                let mut names = HashSet::new();

                for (name, schema) in fields {
                    if name.trim().is_empty() || !names.insert(name) {
                        return Err(format!("field {name:?} is empty or repeated"));
                    }
                    schema
                        .check_at(depth + 1)
                        .map_err(|error| format!("{name}: {error}"))?;
                }

                Ok(())
            }
            Self::Array { items, max_items } => {
                if !(1..=MAX_ITEMS).contains(max_items) {
                    return Err(format!("an array holds 1-{MAX_ITEMS} items"));
                }
                items.check_at(depth + 1)
            }
            Self::String { max_chars } if !(1..=MAX_STRING_CHARS).contains(max_chars) => {
                Err(format!("a string holds 1-{MAX_STRING_CHARS} characters"))
            }
            Self::Enum(values) => {
                let unique: HashSet<&String> = values.iter().collect();

                if values.is_empty()
                    || values.len() > MAX_ENUM_VALUES
                    || unique.len() != values.len()
                {
                    Err(format!("an enum needs 1-{MAX_ENUM_VALUES} distinct values"))
                } else {
                    Ok(())
                }
            }
            Self::Number { min, max } if !(min.is_finite() && max.is_finite() && min <= max) => {
                Err("a number needs finite bounds, min <= max".into())
            }
            _ => Ok(()),
        }
    }

    /// The standard JSON Schema a provider enforces. Evidence IDs become an
    /// enum of `evidence`, so the provider itself keeps citations real.
    #[must_use]
    pub fn to_json_schema(&self, evidence: &[&str]) -> Value {
        match self {
            Self::Object(fields) => {
                let properties: Map<String, Value> = fields
                    .iter()
                    .map(|(name, schema)| (name.clone(), schema.to_json_schema(evidence)))
                    .collect();
                let required: Vec<&String> = fields.iter().map(|(name, _)| name).collect();

                json!({
                    "type": "object",
                    "properties": properties,
                    "required": required,
                    "additionalProperties": false,
                })
            }
            Self::Array { items, max_items } => json!({
                "type": "array",
                "items": items.to_json_schema(evidence),
                "maxItems": max_items,
            }),
            Self::String { max_chars } => json!({ "type": "string", "maxLength": max_chars }),
            Self::Enum(values) => json!({ "type": "string", "enum": values }),
            Self::Boolean => json!({ "type": "boolean" }),
            Self::Number { min, max } => {
                json!({ "type": "number", "minimum": min, "maximum": max })
            }
            Self::EvidenceId => json!({ "type": "string", "enum": evidence }),
        }
    }

    /// Whether `value` fits the schema, citing only `evidence`.
    ///
    /// # Errors
    ///
    /// Where the value first departs from the schema.
    pub fn validate(&self, value: &Value, evidence: &HashSet<&str>) -> Result<(), String> {
        match (self, value) {
            (Self::Object(fields), Value::Object(object)) => {
                if let Some(extra) = object
                    .keys()
                    .find(|key| !fields.iter().any(|(name, _)| name == *key))
                {
                    return Err(format!("unexpected field {extra}"));
                }

                for (name, schema) in fields {
                    let field = object
                        .get(name)
                        .ok_or_else(|| format!("missing field {name}"))?;

                    schema
                        .validate(field, evidence)
                        .map_err(|error| format!("{name}: {error}"))?;
                }

                Ok(())
            }
            (Self::Array { items, max_items }, Value::Array(values)) => {
                if values.len() > *max_items {
                    return Err(format!("more than {max_items} items"));
                }
                values
                    .iter()
                    .try_for_each(|value| items.validate(value, evidence))
            }
            (Self::String { max_chars }, Value::String(text))
                if text.chars().count() > *max_chars =>
            {
                Err(format!("longer than {max_chars} characters"))
            }
            (Self::String { .. }, Value::String(_)) | (Self::Boolean, Value::Bool(_)) => Ok(()),
            (Self::Enum(values), Value::String(text)) if values.contains(text) => Ok(()),
            (Self::Enum(_), Value::String(text)) => {
                Err(format!("{text:?} is not an allowed value"))
            }
            (Self::Number { min, max }, Value::Number(number)) => match number.as_f64() {
                Some(n) if (*min..=*max).contains(&n) => Ok(()),
                _ => Err(format!("{number} is outside [{min}, {max}]")),
            },
            (Self::EvidenceId, Value::String(id)) if evidence.contains(id.as_str()) => Ok(()),
            (Self::EvidenceId, Value::String(id)) => Err(format!("cites unknown evidence {id:?}")),
            (_, other) => Err(format!("unexpected {}", kind(other))),
        }
    }
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Why a reasoning produced no verdict. None of these is a verdict.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "failure", content = "detail", rename_all = "snake_case")]
pub enum ReasoningFailure {
    /// No executor serves the profile, or it is switched off.
    Unavailable(String),
    /// The request's input exceeds its budget; nothing was sent.
    TooLarge,
    /// The model declined to answer.
    Refused,
    /// The output ran into its limit.
    Truncated,
    TimedOut,
    /// The output does not fit the schema.
    Invalid(String),
    /// Transport or provider error.
    Failed(String),
}

impl fmt::Display for ReasoningFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(why) => write!(formatter, "unavailable: {why}"),
            Self::TooLarge => write!(formatter, "input exceeds {MAX_REASONING_INPUT_BYTES} bytes"),
            Self::Refused => write!(formatter, "refused"),
            Self::Truncated => write!(formatter, "truncated"),
            Self::TimedOut => write!(formatter, "timed out"),
            Self::Invalid(why) => write!(formatter, "invalid verdict: {why}"),
            Self::Failed(why) => write!(formatter, "failed: {why}"),
        }
    }
}

/// A reasoning's outcome: a verdict that fits its schema, or why there is none.
pub type Reasoned = Result<Value, ReasoningFailure>;

/// One structured LLM call, as a provider plugin's backend runs it. The
/// host has already rendered the prompt and redacted what it quotes.
#[derive(Clone, Copy, Debug)]
pub struct Completion<'a> {
    pub model: &'a str,
    /// A provider's reasoning effort, such as `low`; `None` is its default.
    pub effort: Option<&'a str>,
    /// Trusted instructions, kept apart from the input.
    pub instructions: &'a str,
    /// The question and its evidence.
    pub input: &'a str,
    /// A name for the output format, such as the profile's.
    pub name: &'a str,
    /// The standard JSON Schema the output must fit, from
    /// [`Schema::to_json_schema`]. A backend enforces it where its provider
    /// can; the host validates the verdict either way.
    pub schema: &'a Value,
    pub max_output_tokens: u32,
    pub timeout: Duration,
}

/// A provider plugin's way to run a [`Completion`]: one model call whose
/// output is JSON, or an explicit failure.
pub trait Backend: Send {
    fn complete(&mut self, completion: &Completion<'_>) -> Reasoned;
}

/// An LLM that reasons. Implementations answer one request per call; the
/// engine validates the verdict before any capability sees it.
pub trait SystemTwo: Send {
    fn name(&self) -> &str;

    fn reason(&mut self, request: &Reasoning) -> Reasoned;
}

impl SystemTwo for Box<dyn SystemTwo> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn reason(&mut self, request: &Reasoning) -> Reasoned {
        (**self).reason(request)
    }
}

/// Run `request` on `executor` within the input and output budgets, and
/// accept only a verdict that fits its schema.
pub fn run(executor: Option<&mut dyn SystemTwo>, request: &Reasoning) -> Reasoned {
    if let Err(error) = request.schema.check() {
        return Err(ReasoningFailure::Invalid(format!("schema: {error}")));
    }
    if request.input_bytes() > MAX_REASONING_INPUT_BYTES {
        return Err(ReasoningFailure::TooLarge);
    }

    let Some(executor) = executor else {
        return Err(ReasoningFailure::Unavailable("System Two is off".into()));
    };
    let verdict = executor.reason(request)?;

    validate_verdict(request, &verdict)?;

    Ok(verdict)
}

/// Check a verdict against its request's schema, evidence and size bound.
///
/// # Errors
///
/// [`ReasoningFailure::Invalid`] naming the first problem.
pub fn validate_verdict(request: &Reasoning, verdict: &Value) -> Result<(), ReasoningFailure> {
    if verdict.to_string().len() > MAX_VERDICT_BYTES {
        return Err(ReasoningFailure::Invalid(format!(
            "exceeds {MAX_VERDICT_BYTES} bytes"
        )));
    }

    request
        .schema
        .validate(verdict, &request.evidence_ids())
        .map_err(ReasoningFailure::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review() -> Reasoning {
        Reasoning {
            id: "review".into(),
            profile: "completion-review".into(),
            instructions: "Is the goal done?".into(),
            evidence: vec![Evidence {
                id: "tests".into(),
                text: "cargo test: 41 passed".into(),
            }],
            activity: false,
            schema: Schema::Object(vec![
                (
                    "verdict".into(),
                    Schema::Enum(vec!["complete".into(), "continue".into()]),
                ),
                (
                    "cites".into(),
                    Schema::Array {
                        items: Box::new(Schema::EvidenceId),
                        max_items: 4,
                    },
                ),
            ]),
        }
    }

    struct Fixed(Reasoned);

    impl SystemTwo for Fixed {
        fn name(&self) -> &str {
            "fixed"
        }

        fn reason(&mut self, _: &Reasoning) -> Reasoned {
            self.0.clone()
        }
    }

    fn outcome(reply: Reasoned) -> Reasoned {
        run(Some(&mut Fixed(reply)), &review())
    }

    #[test]
    fn a_verdict_must_fit_its_schema_and_cite_only_supplied_evidence() {
        let good = json!({ "verdict": "complete", "cites": ["tests"] });

        assert_eq!(outcome(Ok(good.clone())), Ok(good));
        for bad in [
            json!({ "verdict": "done", "cites": [] }),
            json!({ "verdict": "complete", "cites": ["made-up"] }),
            json!({ "verdict": "complete" }),
            json!({ "verdict": "complete", "cites": [], "extra": 1 }),
            json!("complete"),
        ] {
            assert!(
                matches!(outcome(Ok(bad.clone())), Err(ReasoningFailure::Invalid(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn failures_pass_through_and_nothing_runs_without_an_executor_or_within_budget() {
        assert_eq!(
            outcome(Err(ReasoningFailure::Refused)),
            Err(ReasoningFailure::Refused)
        );
        assert!(matches!(
            run(None, &review()),
            Err(ReasoningFailure::Unavailable(_))
        ));

        let mut large = review();
        large.evidence[0].text = "x".repeat(MAX_REASONING_INPUT_BYTES);
        assert_eq!(
            run(Some(&mut Fixed(Ok(json!({})))), &large),
            Err(ReasoningFailure::TooLarge)
        );
    }

    #[test]
    fn schemas_are_bounded_and_evidence_ids_become_an_enum() {
        assert!(Schema::Enum(vec![]).check().is_err());
        assert!(
            Schema::Object(vec![
                ("a".into(), Schema::Boolean),
                ("a".into(), Schema::Boolean)
            ])
            .check()
            .is_err()
        );
        assert!(Schema::String { max_chars: 0 }.check().is_err());
        assert!(Schema::Number { min: 1.0, max: 0.0 }.check().is_err());

        let deep = (0..6).fold(Schema::Boolean, |inner, _| Schema::Array {
            items: Box::new(inner),
            max_items: 1,
        });
        assert!(deep.check().is_err());

        assert_eq!(
            review().schema.to_json_schema(&["tests"])["properties"]["cites"]["items"],
            json!({ "type": "string", "enum": ["tests"] })
        );
        assert_eq!(
            review().schema.to_json_schema(&[])["required"],
            json!(["verdict", "cites"])
        );
    }
}
