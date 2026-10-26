//! Rules read one answer; they are the only place thresholds live. Every rule
//! refuses a failed call.

use serde::{Deserialize, Serialize};

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

/// A configured bar: P(yes) `at`, with at least `confidence`. Both are in
/// `[0, 1]`, checked when read; a capability chooses which rule it builds.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "RawThreshold", into = "RawThreshold")]
pub struct Threshold {
    at: f32,
    confidence: f32,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RawThreshold {
    at: f32,
    confidence: f32,
}

impl TryFrom<RawThreshold> for Threshold {
    type Error = String;

    fn try_from(raw: RawThreshold) -> Result<Self, String> {
        Ok(Self {
            at: unit("at", raw.at)?,
            confidence: unit("confidence", raw.confidence)?,
        })
    }
}

impl From<Threshold> for RawThreshold {
    fn from(threshold: Threshold) -> Self {
        Self {
            at: threshold.at,
            confidence: threshold.confidence,
        }
    }
}

impl Threshold {
    /// [`Rule::yes`] at this bar.
    #[must_use]
    pub const fn yes(self) -> Rule {
        Rule::yes(self.at, self.confidence)
    }

    /// [`Rule::no`] at this bar.
    #[must_use]
    pub const fn no(self) -> Rule {
        Rule::no(self.at, self.confidence)
    }

    /// [`Rule::unless_no`] at this bar.
    #[must_use]
    pub const fn unless_no(self) -> Rule {
        Rule::unless_no(self.at, self.confidence)
    }
}

/// A configured confidence in `[0, 1]`, checked when read.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(try_from = "f32", into = "f32")]
pub struct Confidence(f32);

impl TryFrom<f32> for Confidence {
    type Error = String;

    fn try_from(value: f32) -> Result<Self, String> {
        unit("confidence", value).map(Self)
    }
}

impl From<Confidence> for f32 {
    fn from(confidence: Confidence) -> Self {
        confidence.0
    }
}

impl Confidence {
    /// [`Rule::pick`] at this confidence.
    #[must_use]
    pub const fn pick(self) -> Pick {
        Rule::pick(self.0)
    }

    #[must_use]
    pub const fn value(self) -> f32 {
        self.0
    }
}

fn unit(field: &str, value: f32) -> Result<f32, String> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{field} {value} is outside [0, 1]"))
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
