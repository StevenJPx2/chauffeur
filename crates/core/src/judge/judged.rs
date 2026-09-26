//! A capability that judges through a [`Judge`]: it composes one from a
//! signal and acts on the value it finishes with. [`Judging`] adapts it to
//! [`Capability`], holding the judge between rounds.

use crate::capability::{Capability, PipeStep, Plan};
use crate::effect::Effect;
use crate::engine::MAX_ROUNDS;
use crate::signal::Signal;
use crate::situation::Situation;
use crate::system_one::Answer;

use super::Judge;

pub trait Judged: Send {
    /// What a finished judge hands to [`Judged::act`].
    type Verdict: Send + 'static;

    /// Unique capability ID.
    fn id(&self) -> &str;

    /// The judge for `signal`, or `None` when the signal is not this
    /// capability's concern. A judge that is already done asks nothing.
    fn judge(&mut self, situation: &Situation, signal: &Signal) -> Option<Judge<Self::Verdict>>;

    /// Turn the finished judge's value into effects.
    fn act(&mut self, signal: &Signal, verdict: Self::Verdict) -> Vec<Effect>;

    /// Per-agent memory to keep across daemon restarts.
    fn save(&self) -> Option<serde_json::Value> {
        None
    }

    /// Restore what [`Judged::save`] returned.
    fn load(&mut self, _state: serde_json::Value) {}
}

/// A [`Judged`] capability as the engine drives it.
pub struct Judging<C: Judged> {
    capability: C,
    /// The judge between rounds of the current signal.
    pending: Option<Judge<C::Verdict>>,
}

impl<C: Judged> Judging<C> {
    #[must_use]
    pub fn new(capability: C) -> Self {
        Self {
            capability,
            pending: None,
        }
    }

    /// The judged capability, such as for its own inspection in tests.
    #[must_use]
    pub fn inner(&self) -> &C {
        &self.capability
    }

    /// Act on a finished judge.
    fn finish(&mut self, signal: &Signal, verdict: C::Verdict) -> PipeStep {
        PipeStep::Done(self.capability.act(signal, verdict))
    }
}

impl<C: Judged + Default> Default for Judging<C> {
    fn default() -> Self {
        Self::new(C::default())
    }
}

impl<C: Judged> Capability for Judging<C> {
    fn id(&self) -> &str {
        self.capability.id()
    }

    fn plan(&mut self, situation: &Situation, signal: &Signal) -> Plan {
        self.pending = None;

        match self.capability.judge(situation, signal) {
            None => Plan::Skip,
            Some(Judge::Done(verdict)) => match self.capability.act(signal, verdict) {
                effects if effects.is_empty() => Plan::Skip,
                effects => Plan::Settled(effects),
            },
            Some(judge) => {
                let questions = judge.questions().to_vec();

                self.pending = Some(judge);

                Plan::Ask(questions)
            }
        }
    }

    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        match self.advance(signal, answers, MAX_ROUNDS) {
            PipeStep::Done(effects) => effects,
            PipeStep::Next(_) => Vec::new(),
        }
    }

    /// Step the judge. It asks again only when this round's call succeeded
    /// and rounds remain, since the engine runs no further round otherwise;
    /// in every other case it settles, so its value is always acted on.
    fn advance(&mut self, signal: &Signal, answers: Option<&[Answer]>, round: usize) -> PipeStep {
        let Some(judge) = self.pending.take() else {
            return PipeStep::Done(Vec::new());
        };

        match judge.step(answers) {
            Judge::Done(verdict) => self.finish(signal, verdict),
            judge if answers.is_none() || round >= MAX_ROUNDS => {
                let verdict = judge.settle();

                self.finish(signal, verdict)
            }
            judge => {
                let questions = judge.questions().to_vec();

                self.pending = Some(judge);

                PipeStep::Next(questions)
            }
        }
    }

    fn save(&self) -> Option<serde_json::Value> {
        self.capability.save()
    }

    fn load(&mut self, state: serde_json::Value) {
        self.capability.load(state);
    }
}
