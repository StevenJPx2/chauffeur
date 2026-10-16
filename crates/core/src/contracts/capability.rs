//! Decoupled capabilities: each turns routing answers into effects.

use crate::effect::Effect;
use crate::signal::Signal;
use crate::situation::Situation;
use crate::system_one::{Answer, Question};
use crate::system_two::{Reasoned, Reasoning, ReasoningFailure};

/// What a capability needs for one signal.
#[derive(Debug, PartialEq)]
pub enum Plan {
    /// Not interested in this signal.
    Skip,
    /// Effects fixed by facts alone, such as a veto or an empty candidate set.
    Settled(Vec<Effect>),
    /// Needs System One judgments. Question IDs are local to the capability
    /// and must be unique within the plan.
    Ask(Vec<Question>),
    /// Needs System One judgments, System Two reasoning, or both.
    Infer(Inference),
}

/// The next step of a capability's bounded judgment pipe.
#[derive(Debug, PartialEq)]
pub enum PipeStep {
    Done(Vec<Effect>),
    Next(Vec<Question>),
    /// Another round that may reason with System Two.
    Infer(Inference),
}

/// What one round asks: System One questions, answered in one batched call,
/// and System Two reasonings, each its own call.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inference {
    pub questions: Vec<Question>,
    pub reasonings: Vec<Reasoning>,
}

impl Inference {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.questions.is_empty() && self.reasonings.is_empty()
    }
}

impl From<Vec<Question>> for Inference {
    fn from(questions: Vec<Question>) -> Self {
        Self {
            questions,
            reasonings: Vec::new(),
        }
    }
}

/// What one round brought back.
#[derive(Clone, Copy, Debug, Default)]
pub struct Round<'a> {
    /// System One's answers; `None` when its call failed.
    pub answers: Option<&'a [Answer]>,
    /// Each reasoning's outcome, by request ID.
    pub reasoned: &'a [(String, Reasoned)],
}

impl<'a> Round<'a> {
    /// A round of System One answers alone.
    #[must_use]
    pub fn answers(answers: Option<&'a [Answer]>) -> Self {
        Self {
            answers,
            reasoned: &[],
        }
    }

    /// The outcome of reasoning `id`; unavailable when it never ran, such as
    /// once rounds run out.
    pub fn reasoned(&self, id: &str) -> Reasoned {
        self.reasoned
            .iter()
            .find(|(asked, _)| asked == id)
            .map_or_else(
                || Err(ReasoningFailure::Unavailable("not run".into())),
                |(_, outcome)| outcome.clone(),
            )
    }
}

pub trait Capability: Send {
    /// Unique capability ID.
    fn id(&self) -> &str;

    fn plan(&mut self, situation: &Situation, signal: &Signal) -> Plan;

    /// Turn the judgments into effects. Answers carry the local question IDs.
    /// `answers` is `None` when System One failed; the capability applies its
    /// failure posture.
    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect>;

    /// Called after a round of answers. One-step capabilities use `decide`.
    fn advance(&mut self, signal: &Signal, answers: Option<&[Answer]>, _round: usize) -> PipeStep {
        PipeStep::Done(self.decide(signal, answers))
    }

    /// Called after a round that may include System Two outcomes, with
    /// local IDs. Capabilities that never reason use `advance`.
    fn advance_round(&mut self, signal: &Signal, outcome: &Round<'_>, round: usize) -> PipeStep {
        self.advance(signal, outcome.answers, round)
    }

    /// Per-agent memory to keep across daemon restarts, as JSON. `None` when
    /// the capability keeps none.
    fn save(&self) -> Option<serde_json::Value> {
        None
    }

    /// Restore what [`Capability::save`] returned. A value that no longer
    /// fits is ignored, so the capability starts fresh.
    fn load(&mut self, _state: serde_json::Value) {}
}
