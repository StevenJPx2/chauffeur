//! What a transition reports, and the final verdict.

use core::fmt;

use crate::plan::Outcome;

/// One thing that happened during a transition, in the order it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event<O> {
    /// A valid plan of `steps` steps began.
    Started { steps: usize },
    /// Round `round` asks these steps, in plan order.
    RoundReady { round: u32, steps: Vec<String> },
    /// A step asked in round `round` received its outcome.
    StepSettled {
        round: u32,
        step: String,
        outcome: Outcome<O>,
    },
    /// A step's gates ruled it out; it will never be asked.
    StepSkipped { step: String },
    /// Every step is settled or skipped, after `rounds` rounds.
    Finished { rounds: u32 },
}

/// How a step ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution<O> {
    /// It was asked and answered, or failed.
    Settled(Outcome<O>),
    /// Its gates ruled it out.
    Skipped,
}

/// Every step's resolution, in plan order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict<O> {
    pub(crate) steps: Vec<(String, Resolution<O>)>,
}

impl<O> Verdict<O> {
    /// How `step` ended, if the plan has a step of that ID.
    #[must_use]
    pub fn get(&self, step: &str) -> Option<&Resolution<O>> {
        self.steps
            .iter()
            .find(|(id, _)| id == step)
            .map(|(_, resolution)| resolution)
    }

    /// Each step's ID and resolution, in plan order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Resolution<O>)> {
        self.steps
            .iter()
            .map(|(id, resolution)| (id.as_str(), resolution))
    }
}

impl<O: fmt::Display> fmt::Display for Outcome<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => value.fmt(f),
            Self::Failed => f.write_str("failed"),
        }
    }
}

impl<O: fmt::Display> fmt::Display for Resolution<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settled(outcome) => outcome.fmt(f),
            Self::Skipped => f.write_str("skipped"),
        }
    }
}

impl<O: fmt::Display> fmt::Display for Event<O> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Started { steps } => {
                write!(f, "started a plan of {steps} step{}", plural(*steps))
            }
            Self::RoundReady { round, steps } => {
                write!(f, "round {round} asks {}", steps.join(", "))
            }
            Self::StepSettled {
                round,
                step,
                outcome,
            } => write!(f, "round {round}: {step} settled {outcome}"),
            Self::StepSkipped { step } => write!(f, "{step} skipped"),
            Self::Finished { rounds } => {
                write!(f, "finished after {rounds} round{}", plural(*rounds))
            }
        }
    }
}

fn plural<N: Copy + PartialEq + From<u8>>(count: N) -> &'static str {
    if count == N::from(1) { "" } else { "s" }
}
