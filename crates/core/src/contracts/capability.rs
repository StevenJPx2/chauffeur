//! Decoupled capabilities: each turns routing answers into effects.

use std::fmt;

use crate::effect::Effect;
use crate::engine::MAX_ROUNDS;
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

/// One signal's judgment while it is in flight. The engine numbers runs from
/// 1; several can be in flight at once, so a capability keeps whatever it
/// holds between rounds under its run, never in one shared slot.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RunId(pub u64);

impl RunId {
    /// The run [`Capability::plan`] and [`Capability::decide`] use, for
    /// driving one capability one signal at a time.
    pub const SOLO: Self = Self(0);
}

impl fmt::Display for RunId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "run {}", self.0)
    }
}

pub trait Capability: Send {
    /// Unique capability ID.
    fn id(&self) -> &str;

    /// Plan `run`'s signal. Whatever a capability needs in later rounds it
    /// keeps under `run`.
    fn start(&mut self, run: RunId, situation: &Situation, signal: &Signal) -> Plan;

    /// Continue `run` with its latest round, local IDs restored: finish with
    /// effects, or ask another round. A failed System One call arrives as
    /// `outcome.answers == None`; the capability applies its failure posture.
    fn resume(
        &mut self,
        run: RunId,
        signal: &Signal,
        outcome: &Round<'_>,
        round: usize,
    ) -> PipeStep;

    /// Forget `run`: it was abandoned, or has settled. Its effects are never
    /// asked for.
    fn cancel(&mut self, _run: RunId) {}

    /// [`Capability::start`] for one signal at a time.
    fn plan(&mut self, situation: &Situation, signal: &Signal) -> Plan {
        self.start(RunId::SOLO, situation, signal)
    }

    /// Finish the signal [`Capability::plan`] started with these answers,
    /// settling whatever would have asked again.
    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        match self.resume(RunId::SOLO, signal, &Round::answers(answers), MAX_ROUNDS) {
            PipeStep::Done(effects) => effects,
            PipeStep::Next(_) | PipeStep::Infer(_) => {
                self.cancel(RunId::SOLO);
                Vec::new()
            }
        }
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
