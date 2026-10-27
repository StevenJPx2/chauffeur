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
