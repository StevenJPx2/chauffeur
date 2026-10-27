use chauffeur_core::Outcome;
use chauffeur_judges::{Answer, Settled};
use serde_json::{Map, Value, json};

/// Each step's answer or failure, for the log.
pub(super) fn steps(verdict: &[(&str, &Outcome<Answer>)]) -> Value {
    let steps = verdict
        .iter()
        .map(|(step, outcome)| match outcome {
            Outcome::Value(answer) => json!({ "step": step, "answer": answer_json(answer) }),
            Outcome::Failed(failure) => json!({ "step": step, "failed": failure.name() }),
        })
        .collect();
    Value::Array(steps)
}

fn answer_json(answer: &Answer) -> Value {
    let p: Map<String, Value> = answer
        .p()
        .map(|(value, p)| (value.to_owned(), json!(p.to_f64())))
        .collect();
    let settled = match answer.outcome() {
        Settled::Outcome(value) => value.clone(),
        Settled::Unsure => "unsure".to_owned(),
    };
    json!({
        "outcome": settled,
        "p": p,
        "level": answer.level(),
        "model": answer.by().model,
        "placeholder": answer.by().placeholder,
    })
}
