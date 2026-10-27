use std::collections::BTreeMap;
use std::fmt::Write as _;

use chauffeur_core::Failure;
use serde::Deserialize;

use crate::confidence::Probabilities;
use crate::format::Evidence;
use crate::question::Question;

/// A `/v1/systemone` reply: an answer per question ID.
#[derive(Deserialize)]
pub(super) struct Response {
    pub(super) answers: BTreeMap<String, WireAnswer>,
}

#[derive(Deserialize)]
pub(super) struct WireAnswer {
    #[serde(rename = "type")]
    kind: String,
    noul: Option<f64>,
}

/// Whether the API can ask `question`: its outcomes are exactly `yes` and `no`.
pub(super) fn is_noul(question: &Question) -> bool {
    let outcomes = question.outcomes();
    outcomes.len() == 2 && outcomes.position("yes").is_some() && outcomes.position("no").is_some()
}

/// The evidence the round's questions name, each block once.
pub(super) fn state(questions: &[&Question], evidence: &Evidence) -> String {
    let mut state = String::new();
    for (name, content) in evidence.named_by(questions) {
        let _ = write!(state, "{name}:\n{content}\n\n");
    }
    state
}

/// The probabilities in a `noul` answer, the chance of `yes`.
pub(super) fn read_noul(
    question: &Question,
    answer: Option<&WireAnswer>,
) -> Result<Probabilities, Failure> {
    match answer {
        Some(WireAnswer {
            kind,
            noul: Some(yes),
        }) if kind == "noul" && (0.0..=1.0).contains(yes) => {
            Probabilities::read(question.outcomes(), [("yes", *yes), ("no", 1.0 - yes)])
        }
        _ => Err(Failure::Invalid),
    }
}
