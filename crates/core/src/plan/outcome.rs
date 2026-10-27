use std::fmt;

/// Why a judge gave no answer. A failure is never an answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Failure {
    /// The judge could not be reached, or has no model to ask.
    Unavailable,
    /// The judge's provider refused for now.
    RateLimited,
    /// The judge took longer than its timeout.
    Timeout,
    /// The judge replied with something that is not an answer to the question.
    Invalid,
    /// The run's deadline passed first.
    Deadline,
}

impl Failure {
    /// The reason named `name` on the wire, such as `rate_limited`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [
            Self::Unavailable,
            Self::RateLimited,
            Self::Timeout,
            Self::Invalid,
            Self::Deadline,
        ]
        .into_iter()
        .find(|failure| failure.name() == name)
    }

    /// Whether the tier's next model should answer in its place: the model
    /// could not answer, as opposed to answering badly.
    #[must_use]
    pub const fn falls_back(self) -> bool {
        matches!(self, Self::Unavailable | Self::RateLimited | Self::Timeout)
    }

    /// The reason's wire name, such as `rate_limited`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::RateLimited => "rate_limited",
            Self::Timeout => "timeout",
            Self::Invalid => "invalid",
            Self::Deadline => "deadline",
        }
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl std::error::Error for Failure {}

/// A step's result: the judge's answer, or why it failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<O> {
    /// The judge answered.
    Value(O),
    /// The judge failed.
    Failed(Failure),
}
