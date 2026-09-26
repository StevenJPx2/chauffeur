//! A judgment as a state machine. A [`Judge`] either has finished with a
//! value or is asking System One questions; each round's answers, or `None`
//! when the call failed, move it to its next state. Four primitives build
//! every judge: [`Judge::done`], [`Judge::ask`], [`Judge::then`], and
//! [`Judge::all`]. Judging strategies ([`strategy`]) are compositions of them.

mod judged;
mod rule;
pub mod strategy;

pub use judged::{Judged, Judging};
pub use rule::{NONE, Pick, Rule};

use std::collections::HashSet;

use crate::system_one::{Answer, Question};

/// What a judge does with the next round's answers: `None` when the call
/// failed.
type Next<T> = Box<dyn FnOnce(Option<&[Answer]>) -> Judge<T> + Send>;

/// A judgment in progress, or its result.
pub enum Judge<T> {
    /// Finished, with its value.
    Done(T),
    /// Waiting on these questions; the answers choose the next state.
    Asking {
        questions: Vec<Question>,
        next: Next<T>,
    },
}

impl<T: Send + 'static> Judge<T> {
    /// A judge that has finished, asking nothing.
    #[must_use]
    pub fn done(value: T) -> Self {
        Self::Done(value)
    }

    /// One question; `read` turns its answer, or `None` when the call failed,
    /// into the judge's value.
    #[must_use]
    pub fn ask(
        question: Question,
        read: impl FnOnce(Option<&Answer>) -> T + Send + 'static,
    ) -> Self {
        let id = question.id.clone();

        Self::Asking {
            questions: vec![question],
            next: Box::new(move |answers| {
                Self::Done(read(
                    answers.and_then(|answers| answers.iter().find(|answer| answer.id == id)),
                ))
            }),
        }
    }

    /// When this judge finishes with `v`, continue as `f(v)`: its questions
    /// are asked in the following round.
    #[must_use]
    pub fn then<U: Send + 'static>(
        self,
        f: impl FnOnce(T) -> Judge<U> + Send + 'static,
    ) -> Judge<U> {
        match self {
            Self::Done(value) => f(value),
            Self::Asking { questions, next } => Judge::Asking {
                questions,
                next: Box::new(move |answers| next(answers).then(f)),
            },
        }
    }

    /// This judge and `other` side by side, in the same rounds; finishes
    /// with both values once both have.
    #[must_use]
    pub fn zip<U: Send + 'static>(self, other: Judge<U>) -> Judge<(T, U)> {
        match (self, other) {
            (Self::Done(left), Judge::Done(right)) => Judge::Done((left, right)),
            (left, right) => {
                let questions: Vec<Question> = left
                    .questions()
                    .iter()
                    .chain(right.questions())
                    .cloned()
                    .collect();
                let mut ids = HashSet::new();

                debug_assert!(
                    questions
                        .iter()
                        .all(|question| ids.insert(question.id.clone())),
                    "judges run together must ask distinct question IDs"
                );

                Judge::Asking {
                    questions,
                    next: Box::new(move |answers| left.step(answers).zip(right.step(answers))),
                }
            }
        }
    }

    /// This judge's value, or `None` when a round's call failed, or rounds
    /// ran out, before it finished: a judge whose rules all refused can be
    /// told apart from one that was never answered.
    #[must_use]
    pub fn unless_failed(self) -> Judge<Option<T>> {
        match self {
            Self::Done(value) => Judge::Done(Some(value)),
            Self::Asking { questions, next } => Judge::Asking {
                questions,
                next: Box::new(move |answers| match answers {
                    Some(_) => next(answers).unless_failed(),
                    None => next(None).map(|_| None),
                }),
            },
        }
    }

    /// This judge's value, transformed by `f`.
    #[must_use]
    pub fn map<U: Send + 'static>(self, f: impl FnOnce(T) -> U + Send + 'static) -> Judge<U> {
        self.then(|value| Judge::done(f(value)))
    }

    /// The questions this round asks; none once finished.
    #[must_use]
    pub fn questions(&self) -> &[Question] {
        match self {
            Self::Done(_) => &[],
            Self::Asking { questions, .. } => questions,
        }
    }

    /// Move to the next state with this round's answers, or `None` when the
    /// call failed. A finished judge stays finished.
    #[must_use]
    pub fn step(self, answers: Option<&[Answer]>) -> Self {
        match self {
            Self::Done(value) => Self::Done(value),
            Self::Asking { next, .. } => next(answers),
        }
    }

    /// Finish without asking anything more: every remaining round is read
    /// as a failed call. Used once the engine's rounds are spent.
    #[must_use]
    pub fn settle(self) -> T {
        let mut judge = self;

        loop {
            match judge {
                Self::Done(value) => return value,
                Self::Asking { next, .. } => judge = next(None),
            }
        }
    }
}

impl<T: Send + 'static> Judge<Vec<T>> {
    /// Run `judges` side by side: each round asks every one still asking, in
    /// one call. Finishes, in order, once all of them have.
    ///
    /// # Panics
    ///
    /// In debug builds, when two judges ask the same question ID in one
    /// round: their answers could not be told apart. Question IDs are the
    /// caller's to keep unique.
    #[must_use]
    pub fn all(judges: Vec<Judge<T>>) -> Self {
        if judges.iter().all(|judge| matches!(judge, Judge::Done(_))) {
            return Self::Done(judges.into_iter().map(Judge::settle).collect());
        }

        let questions: Vec<Question> = judges
            .iter()
            .flat_map(|judge| judge.questions().iter().cloned())
            .collect();
        let mut ids = HashSet::new();

        debug_assert!(
            questions
                .iter()
                .all(|question| ids.insert(question.id.clone())),
            "judges run together must ask distinct question IDs"
        );

        Self::Asking {
            questions,
            next: Box::new(move |answers| {
                Self::all(
                    judges
                        .into_iter()
                        .map(|judge| judge.step(answers))
                        .collect(),
                )
            }),
        }
    }
}

#[cfg(test)]
mod tests;
