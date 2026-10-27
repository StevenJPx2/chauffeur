use crate::plan::Outcome;

/// What happened while a plan ran, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event<O> {
    /// The run began, with every step's ID in plan order.
    Started {
        /// The plan's step IDs.
        steps: Vec<String>,
    },
    /// A round's steps were asked of their judges.
    RoundReady {
        /// The round's number, from 1.
        round: u32,
        /// The steps asked in it.
        steps: Vec<String>,
    },
    /// A step's judge answered or failed.
    StepSettled {
        /// The step's ID.
        step: String,
        /// Its outcome.
        outcome: Outcome<O>,
    },
    /// Every step settled.
    Finished,
}

/// Every step's outcome, in plan order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict<O> {
    settled: Vec<(String, Outcome<O>)>,
}

impl<O> Verdict<O> {
    pub(crate) const fn new(settled: Vec<(String, Outcome<O>)>) -> Self {
        Self { settled }
    }

    /// The outcome of the step with this ID.
    #[must_use]
    pub fn get(&self, step: &str) -> Option<&Outcome<O>> {
        self.settled
            .iter()
            .find(|(id, _)| id == step)
            .map(|(_, outcome)| outcome)
    }

    /// Each step's ID and outcome, in plan order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Outcome<O>)> {
        self.settled
            .iter()
            .map(|(id, outcome)| (id.as_str(), outcome))
    }

    /// How many steps settled.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.settled.len()
    }

    /// Whether no step settled.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.settled.is_empty()
    }
}

/// A finished run: its events, and the verdict they end in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run<O> {
    /// Every event, in order.
    pub events: Vec<Event<O>>,
    /// Every step's outcome.
    pub verdict: Verdict<O>,
}
