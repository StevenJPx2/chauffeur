//! A judgment as a state machine. A [`Judgment`] either has finished with a
//! value or is waiting on a round: System One questions, System Two
//! reasonings, or both. Each round's outcome moves it to its next state.
//! Five primitives build every judgment: [`Judgment::done`], [`Judgment::ask`],
//! [`Judgment::reason`], [`Judgment::then`], and [`Judgment::all`]. Judging strategies
//! ([`strategy`]) are compositions of them.

mod judged;
mod rule;
pub mod strategy;

pub use judged::{Judged, Judging};
pub use rule::{Confidence, NONE, Pick, Rule, Threshold};

use std::collections::HashSet;

use crate::capability::{Inference, Round};
use crate::system_one::{Answer, Question};
use crate::system_two::{Reasoned, Reasoning};

/// What a judge does with the next round's outcome.
type Next<T> = Box<dyn FnOnce(&Round<'_>) -> Judgment<T> + Send>;

/// A judgment in progress, or its result.
pub enum Judgment<T> {
    /// Finished, with its value.
    Done(T),
    /// Waiting on this round; its outcome chooses the next state.
    Asking {
        questions: Vec<Question>,
        reasonings: Vec<Reasoning>,
        next: Next<T>,
    },
}

impl<T: Send + 'static> Judgment<T> {
    /// A judge that has finished, asking nothing.
    #[must_use]
    pub fn done(value: T) -> Self {
        Self::Done(value)
    }

    /// One System One question; `read` turns its answer, or `None` when the
    /// call failed, into the judge's value.
    #[must_use]
    pub fn ask(
        question: Question,
        read: impl FnOnce(Option<&Answer>) -> T + Send + 'static,
    ) -> Self {
        let id = question.id.clone();

        Self::Asking {
            questions: vec![question],
            reasonings: Vec::new(),
            next: Box::new(move |round| {
                Self::Done(read(
                    round
                        .answers
                        .and_then(|answers| answers.iter().find(|answer| answer.id == id)),
                ))
            }),
        }
    }

    /// One System Two reasoning; `read` turns its verdict, or why there is
    /// none, into the judge's value.
    #[must_use]
    pub fn reason(request: Reasoning, read: impl FnOnce(Reasoned) -> T + Send + 'static) -> Self {
        let id = request.id.clone();

        Self::Asking {
            questions: Vec::new(),
            reasonings: vec![request],
            next: Box::new(move |round| Self::Done(read(round.reasoned(&id)))),
        }
    }

    /// When this judge finishes with `v`, continue as `f(v)`: its requests
    /// go out in the following round.
    #[must_use]
    pub fn then<U: Send + 'static>(
        self,
        f: impl FnOnce(T) -> Judgment<U> + Send + 'static,
    ) -> Judgment<U> {
        match self {
            Self::Done(value) => f(value),
            Self::Asking {
                questions,
                reasonings,
                next,
            } => Judgment::Asking {
                questions,
                reasonings,
                next: Box::new(move |round| next(round).then(f)),
            },
        }
    }

    /// This judge and `other` side by side, in the same rounds; finishes
    /// with both values once both have.
    #[must_use]
    pub fn zip<U: Send + 'static>(self, other: Judgment<U>) -> Judgment<(T, U)> {
        match (self, other) {
            (Self::Done(left), Judgment::Done(right)) => Judgment::Done((left, right)),
            (left, right) => {
                let questions = joined(left.questions(), right.questions());
                let reasonings = joined(left.reasonings(), right.reasonings());

                debug_assert!(
                    distinct(&questions, &reasonings),
                    "judges run together must ask distinct IDs"
                );

                Judgment::Asking {
                    questions,
                    reasonings,
                    next: Box::new(move |round| {
                        left.step_round(round).zip(right.step_round(round))
                    }),
                }
            }
        }
    }

    /// This judge's value, or `None` when a round's System One call failed,
    /// or rounds ran out, before it finished: a judge whose rules all refused
    /// can be told apart from one that was never answered.
    #[must_use]
    pub fn unless_failed(self) -> Judgment<Option<T>> {
        match self {
            Self::Done(value) => Judgment::Done(Some(value)),
            Self::Asking {
                questions,
                reasonings,
                next,
            } => Judgment::Asking {
                questions,
                reasonings,
                next: Box::new(move |round| match round.answers {
                    Some(_) => next(round).unless_failed(),
                    None => next(round).map(|_| None),
                }),
            },
        }
    }

    /// This judge's value, transformed by `f`.
    #[must_use]
    pub fn map<U: Send + 'static>(self, f: impl FnOnce(T) -> U + Send + 'static) -> Judgment<U> {
        self.then(|value| Judgment::done(f(value)))
    }

    /// The System One questions this round asks; none once finished.
    #[must_use]
    pub fn questions(&self) -> &[Question] {
        match self {
            Self::Done(_) => &[],
            Self::Asking { questions, .. } => questions,
        }
    }

    /// The System Two reasonings this round asks; none once finished.
    #[must_use]
    pub fn reasonings(&self) -> &[Reasoning] {
        match self {
            Self::Done(_) => &[],
            Self::Asking { reasonings, .. } => reasonings,
        }
    }

    /// Everything this round asks.
    #[must_use]
    pub fn inference(&self) -> Inference {
        Inference {
            questions: self.questions().to_vec(),
            reasonings: self.reasonings().to_vec(),
        }
    }

    /// Move to the next state with this round's System One answers, or
    /// `None` when the call failed. A finished judge stays finished.
    #[must_use]
    pub fn step(self, answers: Option<&[Answer]>) -> Self {
        self.step_round(&Round::answers(answers))
    }

    /// Move to the next state with this round's whole outcome.
    #[must_use]
    pub fn step_round(self, round: &Round<'_>) -> Self {
        match self {
            Self::Done(value) => Self::Done(value),
            Self::Asking { next, .. } => next(round),
        }
    }

    /// Finish without asking anything more: every remaining round is read
    /// as failed. Used once the engine's rounds are spent.
    #[must_use]
    pub fn settle(self) -> T {
        let mut judge = self;

        loop {
            match judge {
                Self::Done(value) => return value,
                Self::Asking { next, .. } => judge = next(&Round::default()),
            }
        }
    }
}

impl<T: Send + 'static> Judgment<Vec<T>> {
    /// Run `judges` side by side: each round asks every one still asking,
    /// System One questions in one call. Finishes, in order, once all of
    /// them have.
    ///
    /// # Panics
    ///
    /// In debug builds, when two judges ask the same ID in one round: their
    /// outcomes could not be told apart. IDs are the caller's to keep unique.
    #[must_use]
    pub fn all(judges: Vec<Judgment<T>>) -> Self {
        if judges
            .iter()
            .all(|judge| matches!(judge, Judgment::Done(_)))
        {
            return Self::Done(judges.into_iter().map(Judgment::settle).collect());
        }

        let questions: Vec<Question> = judges
            .iter()
            .flat_map(|judge| judge.questions().iter().cloned())
            .collect();
        let reasonings: Vec<Reasoning> = judges
            .iter()
            .flat_map(|judge| judge.reasonings().iter().cloned())
            .collect();

        debug_assert!(
            distinct(&questions, &reasonings),
            "judges run together must ask distinct IDs"
        );

        Self::Asking {
            questions,
            reasonings,
            next: Box::new(move |round| {
                Self::all(
                    judges
                        .into_iter()
                        .map(|judge| judge.step_round(round))
                        .collect(),
                )
            }),
        }
    }
}

fn joined<T: Clone>(left: &[T], right: &[T]) -> Vec<T> {
    left.iter().chain(right).cloned().collect()
}

/// Whether no ID repeats across a round's questions and reasonings.
fn distinct(questions: &[Question], reasonings: &[Reasoning]) -> bool {
    let mut ids = HashSet::new();

    questions
        .iter()
        .map(|question| &question.id)
        .chain(reasonings.iter().map(|reasoning| &reasoning.id))
        .all(|id| ids.insert(id))
}

#[cfg(test)]
mod tests;
