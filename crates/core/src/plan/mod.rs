mod error;
mod outcome;
mod step;

pub use error::PlanError;
pub use outcome::{Failure, Outcome};
pub use step::Step;

use crate::event::{Event, Run, Verdict};
use crate::judge::Panel;

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
                .any(|earlier| earlier.id() == step.id())
            {
                return Err(PlanError::DuplicateStep(step.id().to_owned()));
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
            if panel.get(step.judge()).is_none() {
                return Err(PlanError::UnknownJudge {
                    step: step.id().to_owned(),
                    judge: step.judge().to_owned(),
                });
            }
            match shares.iter_mut().find(|(name, _)| *name == step.judge()) {
                Some((_, indexes)) => indexes.push(index),
                None => shares.push((step.judge(), vec![index])),
            }
        }

        let ids: Vec<String> = self.steps.iter().map(|step| step.id().to_owned()).collect();
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
                step: step.id().to_owned(),
                outcome: outcome.clone(),
            });
            settled.push((step.id().to_owned(), outcome));
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
            .map(Step::request)
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
