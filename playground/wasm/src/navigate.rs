use std::collections::BTreeMap;

use chauffeur_core::{Failure, Outcome, Panel};
use chauffeur_judges::{Answer, AnsweredBy, Probabilities, Question};
use chauffeur_protocol::{Intent, Navigate, Rule, Strategy, Target, progress};
use serde::Deserialize;
use serde_json::{Value, json};
use wasm_bindgen::prelude::wasm_bindgen;

/// What a stand-in judge answers for one step: a probability per outcome,
/// or a failure.
#[derive(Deserialize)]
#[serde(untagged)]
enum Reply {
    Failed { failed: String },
    P(BTreeMap<String, f64>),
}

#[derive(Deserialize)]
struct Input {
    request: Value,
    /// Rule files by ID, as the daemon would load them.
    #[serde(default)]
    rules: BTreeMap<String, Value>,
    /// Each step's stand-in reply, by step ID.
    #[serde(default)]
    answers: BTreeMap<String, Reply>,
}

/// Navigates as the daemon does, with stand-in replies in place of models.
///
/// Takes `{ request, rules, answers }` as JSON: a `chauffeur.navigate`
/// request's params, rule files by ID, and each step's reply by step ID.
/// Returns `{ events, result }`, where `result` is `{ intents }` or
/// `{ verdict }`; `{ rejected }`, the JSON-RPC error a caller gets; or
/// `{ error }` for input the playground cannot read. A rule that does not
/// load is listed in `refused`.
#[wasm_bindgen]
#[must_use]
pub fn navigate(input: &str) -> String {
    navigate_json(input)
        .unwrap_or_else(|error| json!({ "error": error }))
        .to_string()
}

fn navigate_json(input: &str) -> Result<Value, String> {
    let input: Input = serde_json::from_str(input).map_err(|error| error.to_string())?;
    let request = match Navigate::parse(&input.request) {
        Ok(request) => request,
        Err(rejection) => return Ok(json!({ "rejected": rejection.to_error() })),
    };
    let mut events = Vec::new();
    let result = match (request.target(), request.event()) {
        (Target::Strategy(strategy), _) => {
            let verdict = run(strategy, &input.answers, None, &mut events)?;
            json!({ "verdict": progress::verdict(&verdict) })
        }
        (Target::On(_), Some(event)) => {
            let mut refused = Vec::new();
            let mut intents = Vec::new();
            for (id, text) in &input.rules {
                let rule = match Rule::parse(id, &text.to_string()) {
                    Ok(rule) => rule,
                    Err(error) => {
                        refused.push(error.to_string());
                        continue;
                    }
                };
                if !rule.hooks(&event) {
                    continue;
                }
                events.push(progress::rule_matched(id));
                let verdict = run(rule.strategy(), &input.answers, Some(id), &mut events)?;
                let met = chauffeur_protocol::Then::first_met(rule.then(), &verdict);
                intents.extend(met.map(|then| Intent::new(id, then).to_json()));
            }
            return Ok(
                json!({ "events": events, "result": { "intents": intents }, "refused": refused }),
            );
        }
        (Target::On(_), None) => return Err("an event request has no event".to_owned()),
    };
    Ok(json!({ "events": events, "result": result }))
}

/// Runs `strategy` with each step answered by its stand-in reply, settled
/// under its judge's confidence, and records the run's events.
fn run(
    strategy: &Strategy,
    answers: &BTreeMap<String, Reply>,
    rule: Option<&str>,
    events: &mut Vec<Value>,
) -> Result<chauffeur_core::Verdict<Answer>, String> {
    let step_of = |question: &Question| {
        strategy
            .plan()
            .steps()
            .iter()
            .find(|step| std::ptr::eq(step.request(), question))
            .map(|step| step.id().to_owned())
    };
    let judges: Vec<(&String, _)> = strategy
        .judges()
        .iter()
        .map(|(name, spec)| {
            let judge = move |question: &Question| {
                let reply = step_of(question).and_then(|step| answers.get(&step));
                stand_in(question, reply, &spec.confidence)
            };
            (name, judge)
        })
        .collect();
    let panel = judges.iter().fold(Panel::new(), |panel, (name, judge)| {
        panel.with(name.as_str(), judge)
    });
    let ran = strategy
        .plan()
        .run(&panel)
        .map_err(|error| error.to_string())?;
    events.extend(progress::run_events(rule, &ran.events));
    Ok(ran.verdict)
}

/// A step's stand-in answer: its reply settled under `confidence`, or the
/// failure it names; a step with no reply is `unavailable`, as with no model.
fn stand_in(
    question: &Question,
    reply: Option<&Reply>,
    confidence: &chauffeur_judges::Confidence,
) -> Outcome<Answer> {
    let by = AnsweredBy {
        model: "stand-in".into(),
        placeholder: true,
    };
    match reply {
        None => Outcome::Failed(Failure::Unavailable),
        Some(Reply::Failed { failed }) => {
            Outcome::Failed(Failure::from_name(failed).unwrap_or(Failure::Invalid))
        }
        Some(Reply::P(p)) => {
            match Probabilities::read(
                question.outcomes(),
                p.iter().map(|(value, p)| (value.as_str(), *p)),
            ) {
                Ok(p) => Outcome::Value(Answer::settle(question, p, confidence, by)),
                Err(failure) => Outcome::Failed(failure),
            }
        }
    }
}
