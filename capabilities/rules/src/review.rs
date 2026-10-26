//! Optional review of an unsure or failed step, within two inference rounds.

use chauffeur_core::judgment::strategy::{self, Escalation, Screened};
use chauffeur_core::{Evidence, Judgment, Question, Reasoning, Rule, Schema, SignalKind, Template};
use serde::{Deserialize, Serialize};

use crate::Step;

/// A step's opt-in System Two policy. Wording belongs to its rule file.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    #[serde(default)]
    pub enabled: bool,
    pub when: ReviewWhen,
    pub profile: String,
    pub instructions: String,
    pub holds_on: ReviewVerdict,
}

/// The screened outcomes that warrant a review.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewWhen {
    #[serde(default)]
    pub unsure: bool,
    #[serde(default)]
    pub failed: bool,
}

/// The rules capability's fixed review vocabulary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewVerdict {
    Complete,
    Continue,
    Blocked,
    InsufficientEvidence,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Verdict {
    verdict: ReviewVerdict,
    cites: Vec<String>,
    reason: String,
}

impl Review {
    pub(crate) fn check(&self) -> Result<(), String> {
        if !crate::rule::identifier(&self.profile) {
            return Err("review.profile must be a profile name".into());
        }
        if !self.when.unsure && !self.when.failed {
            return Err("review.when must select unsure or failed".into());
        }
        if self.holds_on == ReviewVerdict::InsufficientEvidence {
            return Err("review.holds_on must be complete, continue or blocked".into());
        }
        Template::new(self.instructions.clone()).check("review.instructions", &["args", "input"])
    }

    pub(crate) fn request(&self, id: &str, kind: &SignalKind) -> Reasoning {
        let evidence = match kind {
            SignalKind::TurnEnd {
                user_request,
                summary,
                todos,
                ..
            } => {
                let mut evidence = Vec::new();
                for (id, text) in [("user_request", user_request), ("summary", summary)] {
                    if !text.trim().is_empty() {
                        evidence.push(Evidence {
                            id: id.into(),
                            text: text.clone(),
                        });
                    }
                }
                evidence.push(Evidence {
                    id: "todos".into(),
                    text: serde_json::json!(todos).to_string(),
                });
                evidence
            }
            _ => Vec::new(),
        };

        Reasoning {
            id: format!("{id}/review"),
            profile: self.profile.clone(),
            instructions: self.instructions.clone(),
            evidence,
            activity: true,
            schema: schema(),
        }
    }
}

fn schema() -> Schema {
    Schema::Object(vec![
        (
            "verdict".into(),
            Schema::Enum(
                ["complete", "continue", "blocked", "insufficient_evidence"]
                    .map(str::to_string)
                    .to_vec(),
            ),
        ),
        (
            "cites".into(),
            Schema::Array {
                items: Box::new(Schema::EvidenceId),
                max_items: 4,
            },
        ),
        ("reason".into(), Schema::String { max_chars: 512 }),
    ])
}

/// Ask steps in order, with reviews only for the configured screened outcomes.
pub(crate) fn chain(
    mut steps: std::vec::IntoIter<(Question, Step, Option<Reasoning>)>,
) -> Judgment<bool> {
    let Some((question, step, request)) = steps.next() else {
        return Judgment::done(true);
    };
    let yes = Rule::yes(step.yes_at_or_above, step.minimum_confidence);
    let judge = match (step.review.filter(|review| review.enabled), request) {
        (Some(review), Some(request)) => strategy::escalate(
            strategy::screen(
                question,
                yes,
                Rule::no(1.0 - step.yes_at_or_above, step.minimum_confidence),
            ),
            Escalation {
                unsure: review.when.unsure,
                failed: review.when.failed,
            },
            |screened| screened == Screened::Yes,
            move |_| {
                Judgment::reason(request, move |outcome| {
                    outcome
                        .ok()
                        .and_then(|value| serde_json::from_value::<Verdict>(value).ok())
                        .is_some_and(|verdict| {
                            verdict.verdict == review.holds_on
                                && !verdict.cites.is_empty()
                                && !verdict.reason.trim().is_empty()
                        })
                })
            },
        ),
        _ => strategy::single(question, yes),
    };

    judge.then(move |held| {
        if held {
            chain(steps)
        } else {
            Judgment::done(false)
        }
    })
}
