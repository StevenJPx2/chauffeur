mod error;
mod label;
mod outcomes;

pub use error::QuestionError;
pub use label::Label;
pub use outcomes::Outcomes;

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
