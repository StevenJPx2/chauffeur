//! What a navigation reports as it runs, and the verdict it ends in, as they
//! go on the wire.

use chauffeur_core::{Event as RunEvent, Outcome, Verdict};
use chauffeur_judges::Answer;
use serde_json::{Map, Value, json};

/// `rule_matched`: a rule's `on` and `when` matched the request.
#[must_use]
pub fn rule_matched(rule: &str) -> Value {
    json!({ "event": "rule_matched", "rule": rule })
}

/// A run's `round_ready` and `step_settled` events, in order, each naming
/// `rule`, or no rule for an inline strategy.
#[must_use]
pub fn run_events(rule: Option<&str>, events: &[RunEvent<Answer>]) -> Vec<Value> {
    events
        .iter()
        .filter_map(|event| {
            let mut fields = match event {
                RunEvent::RoundReady { round, steps } => {
                    json!({ "event": "round_ready", "round": round, "steps": steps })
                }
                RunEvent::StepSettled { step, outcome } => {
                    let mut settled = settled(outcome);
                    if let Some(fields) = settled.as_object_mut() {
                        fields.insert("event".into(), json!("step_settled"));
                        fields.insert("step".into(), json!(step));
                    }
                    settled
                }
                RunEvent::Started { .. } | RunEvent::Finished => return None,
            };
            if let (Some(rule), Some(object)) = (rule, fields.as_object_mut()) {
                object.insert("rule".into(), json!(rule));
            }
            Some(fields)
        })
        .collect()
}

/// A step's result: `{ "answer": { … } }` or `{ "failed": "<reason>" }`.
/// Probabilities and the level have at most three decimals.
#[must_use]
pub fn settled(outcome: &Outcome<Answer>) -> Value {
    match outcome {
        Outcome::Value(answer) => {
            let p: Map<String, Value> = answer
                .p()
                .map(|(value, p)| (value.to_owned(), json!(p.to_f64())))
                .collect();
            let level = (answer.level() * 1000.0).round() / 1000.0;
            json!({ "answer": {
                "outcome": answer.outcome().to_string(),
                "p": p,
                "level": level,
                "model": answer.by().model,
                "placeholder": answer.by().placeholder,
            } })
        }
        Outcome::Failed(failure) => json!({ "failed": failure.name() }),
    }
}

/// Every step's result, by step ID.
#[must_use]
pub fn verdict(verdict: &Verdict<Answer>) -> Value {
    let steps: Map<String, Value> = verdict
        .iter()
        .map(|(step, outcome)| (step.to_owned(), settled(outcome)))
        .collect();
    Value::Object(steps)
}
