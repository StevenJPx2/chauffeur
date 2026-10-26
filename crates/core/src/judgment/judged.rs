//! A capability that judges through a [`Judgment`]: it composes one from a
//! signal and acts on the value it finishes with. [`Judging`] adapts it to
//! [`Capability`], holding the judge between rounds.

use std::collections::HashMap;

use crate::capability::{Capability, Inference, PipeStep, Plan, Round, RunId};
use crate::effect::Effect;
use crate::engine::MAX_ROUNDS;
use crate::signal::Signal;
use crate::situation::Situation;

use super::Judgment;

pub trait Judged: Send {
    /// What a finished judge hands to [`Judged::act`].
    type Verdict: Send + 'static;

    /// Unique capability ID.
    fn id(&self) -> &str;

    /// The judge for `signal`, or `None` when the signal is not this
    /// capability's concern. A judge that is already done asks nothing.
    fn judge(&mut self, situation: &Situation, signal: &Signal) -> Option<Judgment<Self::Verdict>>;

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
    /// Each run's judge between its rounds.
    pending: HashMap<RunId, Judgment<C::Verdict>>,
}

impl<C: Judged> Judging<C> {
    #[must_use]
    pub fn new(capability: C) -> Self {
        Self {
            capability,
            pending: HashMap::new(),
        }
    }

    /// The judged capability, such as for its own inspection in tests.
    #[must_use]
    pub fn inner(&self) -> &C {
        &self.capability
    }

    /// How many runs hold a judge between rounds.
    #[must_use]
    pub fn in_flight(&self) -> usize {
        self.pending.len()
    }

    /// Act on a finished judge.
    fn finish(&mut self, signal: &Signal, verdict: C::Verdict) -> PipeStep {
        PipeStep::Done(self.capability.act(signal, verdict))
    }

    /// Keep `judge` for `run`'s next round, and say what that round asks.
    fn wait(&mut self, run: RunId, judge: Judgment<C::Verdict>) -> Inference {
        let inference = judge.inference();

        self.pending.insert(run, judge);
        inference
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

    fn start(&mut self, run: RunId, situation: &Situation, signal: &Signal) -> Plan {
        self.pending.remove(&run);

        match self.capability.judge(situation, signal) {
            None => Plan::Skip,
            Some(Judgment::Done(verdict)) => match self.capability.act(signal, verdict) {
                effects if effects.is_empty() => Plan::Skip,
                effects => Plan::Settled(effects),
            },
            Some(judge) => match self.wait(run, judge) {
                inference if inference.reasonings.is_empty() => Plan::Ask(inference.questions),
                inference => Plan::Infer(inference),
            },
        }
    }

    /// Step `run`'s judge. It asks again only while rounds remain, and asks
    /// System One again only after System One answered this round, since
    /// the engine runs no further round otherwise; a next round of System
    /// Two alone, such as a fallback for a failed System One call, may run.
    /// In every other case it settles, so its value is always acted on. A
    /// run it holds no judge for, such as one already cancelled, does nothing.
    fn resume(
        &mut self,
        run: RunId,
        signal: &Signal,
        outcome: &Round<'_>,
        round: usize,
    ) -> PipeStep {
        let Some(judge) = self.pending.remove(&run) else {
            return PipeStep::Done(Vec::new());
        };

        match judge.step_round(outcome) {
            Judgment::Done(verdict) => self.finish(signal, verdict),
            judge
                if round >= MAX_ROUNDS
                    || (outcome.answers.is_none() && !judge.questions().is_empty()) =>
            {
                let verdict = judge.settle();

                self.finish(signal, verdict)
            }
            judge => match self.wait(run, judge) {
                inference if inference.reasonings.is_empty() => PipeStep::Next(inference.questions),
                inference => PipeStep::Infer(inference),
            },
        }
    }

    fn cancel(&mut self, run: RunId) {
        self.pending.remove(&run);
    }

    fn save(&self) -> Option<serde_json::Value> {
        self.capability.save()
    }

    fn load(&mut self, state: serde_json::Value) {
        self.capability.load(state);
    }
}
