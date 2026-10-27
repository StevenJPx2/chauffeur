use std::fmt;

/// Why a plan was refused before any judge was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    /// A plan needs at least one step.
    Empty,
    /// Two steps share this ID.
    DuplicateStep(String),
    /// A step names a judge the panel lacks.
    UnknownJudge {
        /// The step's ID.
        step: String,
        /// The judge it names.
        judge: String,
    },
}

impl fmt::Display for PlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a plan needs at least one step"),
            Self::DuplicateStep(id) => write!(f, "two steps are named `{id}`"),
            Self::UnknownJudge { step, judge } => {
                write!(
                    f,
                    "step `{step}` names judge `{judge}`, which the panel lacks"
                )
            }
        }
    }
}

impl std::error::Error for PlanError {}
