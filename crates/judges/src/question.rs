use std::collections::BTreeSet;

/// The fewest and most outcomes a question may have.
const OUTCOMES: std::ops::RangeInclusive<usize> = 2..=64;

/// Names a judge's result can take that no question may use as an outcome.
const RESERVED: [&str; 2] = ["unsure", "failed"];

/// Why a question was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum QuestionError {
    /// The instructions are blank.
    #[error("a question needs instructions")]
    NoInstructions,
    /// Fewer than 2 or more than 64 outcomes.
    #[error("a question needs 2 to 64 outcomes, not {0}")]
    OutcomeCount(usize),
    /// An outcome is blank.
    #[error("an outcome is blank")]
    BlankOutcome,
    /// Two outcomes share a value.
    #[error("the outcome `{0}` appears twice")]
    Duplicate(String),
    /// An outcome uses a reserved name.
    #[error("`{0}` is reserved and cannot be an outcome")]
    Reserved(String),
}

/// One possible outcome: its value, and an optional description the model sees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Label {
    value: String,
    description: Option<String>,
}

impl Label {
    /// An outcome with only its value.
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            description: None,
        }
    }

    /// An outcome with a description the model sees.
    pub fn described(value: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            description: Some(description.into()),
        }
    }

    /// The outcome's value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// The description the model sees, if any.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

impl From<&str> for Label {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// A question's outcomes, in order: 2 to 64, unique, none reserved. The order
/// is a scale's order when the outcomes are one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcomes(Vec<Label>);

impl Outcomes {
    /// Validates `labels` as a question's outcomes.
    ///
    /// # Errors
    ///
    /// A [`QuestionError`] naming the first problem found.
    pub fn new(labels: impl IntoIterator<Item = impl Into<Label>>) -> Result<Self, QuestionError> {
        let labels: Vec<Label> = labels.into_iter().map(Into::into).collect();
        if !OUTCOMES.contains(&labels.len()) {
            return Err(QuestionError::OutcomeCount(labels.len()));
        }
        let mut seen = BTreeSet::new();
        for label in &labels {
            let value = label.value.as_str();
            if value.trim().is_empty() {
                return Err(QuestionError::BlankOutcome);
            }
            if RESERVED.contains(&value) {
                return Err(QuestionError::Reserved(value.to_owned()));
            }
            if !seen.insert(value) {
                return Err(QuestionError::Duplicate(value.to_owned()));
            }
        }
        Ok(Self(labels))
    }

    /// How many outcomes there are.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Always false: a question has at least two outcomes.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The outcomes, in order.
    pub fn iter(&self) -> impl Iterator<Item = &Label> {
        self.0.iter()
    }

    /// The position of the outcome whose value is `value`.
    #[must_use]
    pub fn position(&self, value: &str) -> Option<usize> {
        self.0.iter().position(|label| label.value == value)
    }

    /// The outcome at `index`.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Label> {
        self.0.get(index)
    }
}

/// Instructions, the outcomes to choose from, and the evidence blocks the
/// judge sees. Nothing else reaches a judge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    instructions: String,
    outcomes: Outcomes,
    evidence: Vec<String>,
}

impl Question {
    /// A question with its instructions, outcomes and the evidence blocks it names.
    ///
    /// # Errors
    ///
    /// [`QuestionError::NoInstructions`] when `instructions` is blank.
    pub fn new(
        instructions: impl Into<String>,
        outcomes: Outcomes,
        evidence: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, QuestionError> {
        let instructions = instructions.into();
        if instructions.trim().is_empty() {
            return Err(QuestionError::NoInstructions);
        }
        Ok(Self {
            instructions,
            outcomes,
            evidence: evidence.into_iter().map(Into::into).collect(),
        })
    }

    /// A yes/no question, using Jev's name for it.
    ///
    /// # Errors
    ///
    /// [`QuestionError::NoInstructions`] when `instructions` is blank.
    pub fn noul(
        instructions: impl Into<String>,
        evidence: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, QuestionError> {
        Self::new(instructions, Outcomes::new(["yes", "no"])?, evidence)
    }

    /// What the judge is asked.
    #[must_use]
    pub fn instructions(&self) -> &str {
        &self.instructions
    }

    /// The outcomes to choose from.
    #[must_use]
    pub const fn outcomes(&self) -> &Outcomes {
        &self.outcomes
    }

    /// The names of the evidence blocks the judge sees.
    #[must_use]
    pub fn evidence(&self) -> &[String] {
        &self.evidence
    }
}
