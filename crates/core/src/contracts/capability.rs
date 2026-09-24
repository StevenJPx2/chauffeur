//! Decoupled capabilities: each turns routing answers into effects.

use crate::effect::Effect;
use crate::signal::Signal;
use crate::situation::Situation;
use crate::system_one::{Answer, Question};

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
}

/// The next step of a capability's bounded judgment pipe.
pub enum PipeStep {
    Done(Vec<Effect>),
    Next(Vec<Question>),
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

    /// Per-agent memory to keep across daemon restarts, as JSON. `None` when
    /// the capability keeps none.
    fn save(&self) -> Option<serde_json::Value> {
        None
    }

    /// Restore what [`Capability::save`] returned. A value that no longer
    /// fits is ignored, so the capability starts fresh.
    fn load(&mut self, _state: serde_json::Value) {}
}
