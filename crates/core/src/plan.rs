use std::fmt;

use crate::event::{Event, Run, Verdict};
use crate::judge::Panel;

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

/// One question, `request`, for the judge named `judge`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step<Q> {
    id: String,
    judge: String,
    request: Q,
}

impl<Q> Step<Q> {
    /// A step with its ID, the name of the judge that answers it, and its request.
    pub fn new(id: impl Into<String>, judge: impl Into<String>, request: Q) -> Self {
        Self {
            id: id.into(),
            judge: judge.into(),
            request,
        }
    }

    /// The step's ID, unique within its plan.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The name of the judge that answers it.
    #[must_use]
    pub fn judge(&self) -> &str {
        &self.judge
    }

    /// What the judge is asked.
    #[must_use]
    pub const fn request(&self) -> &Q {
        &self.request
    }
}

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

/// An ordered list of uniquely named steps. Every step is asked in one round.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan<Q> {
    steps: Vec<Step<Q>>,
}

impl<Q> Plan<Q> {
    /// A plan of `steps`, in order.
    ///
    /// # Errors
    ///
    /// [`PlanError::Empty`] with no steps, and [`PlanError::DuplicateStep`]
    /// when two steps share an ID.
    pub fn new(steps: impl IntoIterator<Item = Step<Q>>) -> Result<Self, PlanError> {
        let steps: Vec<Step<Q>> = steps.into_iter().collect();
        if steps.is_empty() {
            return Err(PlanError::Empty);
        }
        for (index, step) in steps.iter().enumerate() {
            if steps
                .iter()
                .take(index)
                .any(|earlier| earlier.id == step.id)
            {
                return Err(PlanError::DuplicateStep(step.id.clone()));
            }
        }
        Ok(Self { steps })
    }

    /// The steps, in order.
    #[must_use]
    pub fn steps(&self) -> &[Step<Q>] {
        &self.steps
    }

    /// Asks every step's judge and returns the events and the verdict. Each
    /// judge answers all of its steps in one `judge_all` call, in the order
    /// the judges first appear in the plan.
    ///
    /// # Errors
    ///
    /// [`PlanError::UnknownJudge`] when a step names a judge the panel lacks;
    /// no judge is asked.
    pub fn run<O: Clone>(&self, panel: &Panel<'_, Q, O>) -> Result<Run<O>, PlanError> {
        let mut shares: Vec<(&str, Vec<usize>)> = Vec::new();
        for (index, step) in self.steps.iter().enumerate() {
            if panel.get(&step.judge).is_none() {
                return Err(PlanError::UnknownJudge {
                    step: step.id.clone(),
                    judge: step.judge.clone(),
                });
            }
            match shares.iter_mut().find(|(name, _)| *name == step.judge) {
                Some((_, indexes)) => indexes.push(index),
                None => shares.push((step.judge.as_str(), vec![index])),
            }
        }

        let ids: Vec<String> = self.steps.iter().map(|step| step.id.clone()).collect();
        let mut events = vec![
            Event::Started { steps: ids.clone() },
            Event::RoundReady {
                round: 1,
                steps: ids,
            },
        ];

        let mut outcomes: Vec<Option<Outcome<O>>> = std::iter::repeat_with(|| None)
            .take(self.steps.len())
            .collect();
        for (name, indexes) in &shares {
            let answers = self.ask_share(panel, name, indexes);
            for (index, outcome) in indexes.iter().zip(answers) {
                if let Some(slot) = outcomes.get_mut(*index) {
                    *slot = Some(outcome);
                }
            }
        }

        let mut settled = Vec::with_capacity(self.steps.len());
        for (step, outcome) in self.steps.iter().zip(outcomes) {
            let outcome = outcome.unwrap_or(Outcome::Failed(Failure::Invalid));
            events.push(Event::StepSettled {
                step: step.id.clone(),
                outcome: outcome.clone(),
            });
            settled.push((step.id.clone(), outcome));
        }
        events.push(Event::Finished);

        Ok(Run {
            events,
            verdict: Verdict::new(settled),
        })
    }

    /// One judge's answers for the steps at `indexes`; a batch of the wrong
    /// length settles every one of them as invalid.
    fn ask_share<O>(
        &self,
        panel: &Panel<'_, Q, O>,
        name: &str,
        indexes: &[usize],
    ) -> Vec<Outcome<O>> {
        let requests: Vec<&Q> = indexes
            .iter()
            .filter_map(|index| self.steps.get(*index))
            .map(|step| &step.request)
            .collect();
        let answers = panel
            .get(name)
            .map(|judge| judge.judge_all(&requests))
            .unwrap_or_default();
        if answers.len() == indexes.len() {
            answers
        } else {
            indexes
                .iter()
                .map(|_| Outcome::Failed(Failure::Invalid))
                .collect()
        }
    }
}
