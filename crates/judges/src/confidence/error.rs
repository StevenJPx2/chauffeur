/// Why a confidence does not fit a question.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ConfidenceError {
    /// The per-outcome confidence lacks this outcome.
    #[error("the confidence has no bar for the outcome `{0}`")]
    Missing(String),
    /// The per-outcome confidence names an outcome the question lacks.
    #[error("the confidence names `{0}`, which is not an outcome")]
    Unknown(String),
}
