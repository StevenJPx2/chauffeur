use chauffeur_core::{Outcome, Verdict};
use chauffeur_judges::{Answer, Settled};
use serde_json::Value;

/// The result a `then` entry needs from one step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expect {
    /// The step settled to this outcome.
    Outcome(String),
    /// The step's answer was not likely enough.
    Unsure,
    /// The step's judge failed, for any reason.
    Failed,
}

impl Expect {
    pub(crate) fn parse(text: &str) -> Self {
        match text {
            "unsure" => Self::Unsure,
            "failed" => Self::Failed,
            outcome => Self::Outcome(outcome.to_owned()),
        }
    }

    fn met_by(&self, outcome: &Outcome<Answer>) -> bool {
        match (self, outcome) {
            (Self::Outcome(expected), Outcome::Value(answer)) => {
                matches!(answer.outcome(), Settled::Outcome(settled) if settled == expected)
            }
            (Self::Unsure, Outcome::Value(answer)) => answer.outcome() == &Settled::Unsure,
            (Self::Failed, Outcome::Failed(_)) => true,
            _ => false,
        }
    }
}

/// One `then` entry: the step results it needs, and the action or intent it
/// returns when all of them hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Then {
    needs: Vec<(String, Expect)>,
    name: String,
    payload: Value,
}

impl Then {
    pub(crate) const fn new(needs: Vec<(String, Expect)>, name: String, payload: Value) -> Self {
        Self {
            needs,
            name,
            payload,
        }
    }

    /// The step results it needs.
    #[must_use]
    pub fn needs(&self) -> &[(String, Expect)] {
        &self.needs
    }

    /// The action or intent's name, such as `message.append` or `withhold`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the action or intent carries.
    #[must_use]
    pub const fn payload(&self) -> &Value {
        &self.payload
    }

    /// The first entry whose needs `verdict` meets, if any.
    #[must_use]
    pub fn first_met<'a>(entries: &'a [Self], verdict: &Verdict<Answer>) -> Option<&'a Self> {
        entries.iter().find(|entry| {
            entry.needs.iter().all(|(step, expect)| {
                verdict
                    .get(step)
                    .is_some_and(|outcome| expect.met_by(outcome))
            })
        })
    }
}
