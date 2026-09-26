//! Rules read one answer; they are the only place thresholds live. Every rule
//! refuses a failed call.

use crate::system_one::{Answer, AnswerValue};

/// The choice option that picks nothing.
pub const NONE: &str = "none";

/// When a yes/no answer holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rule {
    at: f32,
    confidence: f32,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    /// A confident yes: P ≥ `at`.
    Yes,
    /// A confident no: P ≤ `at`.
    No,
    /// Anything but a confident no: not P ≤ `at`.
    UnlessNo,
}

impl Rule {
    /// Holds on P(yes) ≥ `at` with at least `confidence`.
    #[must_use]
    pub const fn yes(at: f32, confidence: f32) -> Self {
        Self {
            at,
            confidence,
            kind: Kind::Yes,
        }
    }

    /// Holds on P(yes) ≤ `at` with at least `confidence`: a confident no,
    /// such as a tool group the task will not need.
    #[must_use]
    pub const fn no(at: f32, confidence: f32) -> Self {
        Self {
            at,
            confidence,
            kind: Kind::No,
        }
    }

    /// Holds unless P(yes) ≤ `at` with at least `confidence`: an unsure
    /// answer holds. Used where the default is to act, such as a project's
    /// skill in its own repository.
    #[must_use]
    pub const fn unless_no(at: f32, confidence: f32) -> Self {
        Self {
            at,
            confidence,
            kind: Kind::UnlessNo,
        }
    }

    /// A choice other than [`NONE`] with at least `confidence`.
    #[must_use]
    pub const fn pick(confidence: f32) -> Pick {
        Pick { confidence }
    }

    /// Whether `answer`, `None` for a failed call, holds.
    #[must_use]
    pub fn holds(&self, answer: Option<&Answer>) -> bool {
        let Some((p, confident)) = answer.and_then(|answer| {
            probability(answer).map(|p| (p, confident(answer, self.confidence)))
        }) else {
            return false;
        };

        match self.kind {
            Kind::Yes => p >= self.at && confident,
            Kind::No => p <= self.at && confident,
            Kind::UnlessNo => !(p <= self.at && confident),
        }
    }
}

/// When a choice answer picks an option.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    confidence: f32,
}

impl Pick {
    /// The chosen option, unless it is [`NONE`], unsure, or the call failed.
    #[must_use]
    pub fn chosen(&self, answer: Option<&Answer>) -> Option<String> {
        let answer = answer?;
        let AnswerValue::Choice(choice) = &answer.value else {
            return None;
        };

        (choice != NONE && confident(answer, self.confidence)).then(|| choice.clone())
    }
}

/// P(yes) of a yes/no answer.
#[must_use]
pub(crate) fn probability(answer: &Answer) -> Option<f32> {
    match answer.value {
        AnswerValue::Noul(p) => Some(p),
        _ => None,
    }
}

fn confident(answer: &Answer, minimum: f32) -> bool {
    answer.effective_confidence() >= minimum
}
