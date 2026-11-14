use chauffeur_core::{Outcome, Run};
use chauffeur_judges::Answer;
use chauffeur_runtime::Ran;
use serde_json::{Map, Value, json};

/// Writes how a run ended into its log `entry`, and gives back its run if
/// it settled.
pub(super) fn ended(entry: &mut Map<String, Value>, ran: Ran) -> Option<Run<Answer>> {
    match ran {
        Ran::Settled(run) => {
            entry.insert(
                "steps".into(),
                steps(&run.verdict.iter().collect::<Vec<_>>()),
            );
            Some(run)
        }
        Ran::Refused(reason) => {
            entry.insert("refused".into(), json!(reason));
            None
        }
        Ran::Deadline => {
            entry.insert("deadline".into(), json!(true));
            None
        }
        Ran::Cancelled => {
            entry.insert("cancelled".into(), json!(true));
            None
        }
    }
}

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
    json!({
        "outcome": answer.outcome().to_string(),
        "p": p,
        "level": answer.level(),
        "model": answer.by().model,
        "placeholder": answer.by().placeholder,
    })
}
