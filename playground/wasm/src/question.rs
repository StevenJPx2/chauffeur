use std::collections::BTreeMap;

use chauffeur_judges::{
    Answer, AnsweredBy, Confidence, Milli, Outcomes, Probabilities, Question, Settled,
};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::wasm_bindgen;

#[derive(Deserialize)]
#[serde(untagged)]
enum ConfidenceInput {
    Bar(f64),
    PerOutcome(BTreeMap<String, f64>),
}

#[derive(Deserialize)]
struct Input {
    outcomes: Vec<String>,
    p: BTreeMap<String, f64>,
    confidence: Option<ConfidenceInput>,
}

/// Settles a question's probabilities under a confidence, as the daemon does.
///
/// Takes `{ outcomes, p, confidence }` as JSON and returns
/// `{ answer: { outcome, p: [{ outcome, p }] in the question's order, level } }`,
/// `{ failed }` for a reply that is not
/// an answer, or `{ error }` for a question or confidence a rule would refuse.
#[wasm_bindgen]
#[must_use]
pub fn settle(input: &str) -> String {
    settle_json(input)
        .unwrap_or_else(|error| json!({ "error": error }))
        .to_string()
}

fn settle_json(input: &str) -> Result<Value, String> {
    let input: Input = serde_json::from_str(input).map_err(|error| error.to_string())?;
    let outcomes = Outcomes::new(input.outcomes.iter().map(String::as_str))
        .map_err(|error| error.to_string())?;
    let question =
        Question::new("playground", outcomes, ["evidence"]).map_err(|error| error.to_string())?;
    let confidence = confidence(input.confidence)?;
    confidence
        .check(question.outcomes())
        .map_err(|error| error.to_string())?;
    let p = match Probabilities::read(
        question.outcomes(),
        input.p.iter().map(|(value, p)| (value.as_str(), *p)),
    ) {
        Ok(p) => p,
        Err(failure) => return Ok(json!({ "failed": failure.name() })),
    };
    let by = AnsweredBy {
        model: "playground".into(),
        placeholder: true,
    };
    let answer = Answer::settle(&question, p, &confidence, by);
    let outcome = match answer.outcome() {
        Settled::Outcome(value) => value.clone(),
        Settled::Unsure => "unsure".to_owned(),
    };
    let p: Vec<Value> = answer
        .p()
        .map(|(value, p)| json!({ "outcome": value, "p": p.to_f64() }))
        .collect();
    Ok(json!({ "answer": { "outcome": outcome, "p": p, "level": answer.level() } }))
}

fn confidence(input: Option<ConfidenceInput>) -> Result<Confidence, String> {
    let milli = |bar: f64| {
        Milli::from_f64(bar)
            .ok_or_else(|| format!("confidence {bar} is not a probability from 0 to 1"))
    };
    Ok(match input {
        None => Confidence::Likeliest,
        Some(ConfidenceInput::Bar(bar)) => Confidence::Bar(milli(bar)?),
        Some(ConfidenceInput::PerOutcome(bars)) => Confidence::PerOutcome(
            bars.into_iter()
                .map(|(outcome, bar)| milli(bar).map(|bar| (outcome, bar)))
                .collect::<Result<_, _>>()?,
        ),
    })
}
