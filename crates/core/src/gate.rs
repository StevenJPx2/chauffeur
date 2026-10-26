//! The index-level kernel: how gates decide and how one pass settles a plan's
//! pending steps. The Kani proofs target this module.

use crate::plan::{Mode, Outcome};

/// Where a step is in its life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepState<O> {
    /// Not yet asked or skipped; its gates are undecided.
    Pending,
    /// In the current round, awaiting an answer.
    Asked,
    Settled(Outcome<O>),
    /// Its gates ruled it out. A skipped step never matches a gate.
    Skipped,
}

impl<O> StepState<O> {
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Settled(_) | Self::Skipped)
    }
}

/// A validated gate: sources are indices of preceding steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link<O> {
    pub mode: Mode,
    pub sources: Vec<usize>,
    pub on: Vec<Outcome<O>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Wait,
    Run,
    Skip,
}

/// Decide a gate from its sources' states, stopping once the result is known.
pub fn decide<'a, O: PartialEq + 'a>(
    mode: Mode,
    on: &[Outcome<O>],
    sources: impl IntoIterator<Item = &'a StepState<O>>,
) -> Decision {
    let mut all_matched = true;
    let mut all_terminal = true;
    for state in sources {
        let matched = matches!(state, StepState::Settled(outcome) if on.contains(outcome));
        let terminal = state.is_terminal();
        match mode {
            Mode::Any if matched => return Decision::Run,
            Mode::All if terminal && !matched => return Decision::Skip,
            Mode::Any | Mode::All => {}
        }
        all_matched &= matched;
        all_terminal &= terminal;
    }
    match mode {
        Mode::Any if all_terminal => Decision::Skip,
        Mode::All if all_matched => Decision::Run,
        Mode::Any | Mode::All => Decision::Wait,
    }
}

/// A step's gates combine with AND: one skip skips it, all runs run it.
/// `before` holds the states of the steps preceding it; a source outside
/// `before` counts as skipped.
pub fn step_decision<O: PartialEq>(gates: &[Link<O>], before: &[StepState<O>]) -> Decision {
    let skipped = StepState::Skipped;
    let mut result = Decision::Run;
    for gate in gates {
        let states = gate
            .sources
            .iter()
            .map(|&source| before.get(source).unwrap_or(&skipped));
        match decide(gate.mode, &gate.on, states) {
            Decision::Skip => return Decision::Skip,
            Decision::Wait => result = Decision::Wait,
            Decision::Run => {}
        }
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Asked(usize),
    Skipped(usize),
}

/// One forward pass in plan order: mark each pending step whose gates pass as
/// asked, and each one they rule out as skipped. Gates only name preceding
/// steps, so a skip propagates to later steps within the same pass.
pub fn resolve<O: PartialEq>(gates: &[Vec<Link<O>>], states: &mut [StepState<O>]) -> Vec<Change> {
    // At most one change per step.
    let mut changes = Vec::with_capacity(states.len());
    for index in 0..states.len() {
        let (before, rest) = states.split_at_mut(index);
        let Some(state) = rest.first_mut() else { break };
        if !matches!(state, StepState::Pending) {
            continue;
        }
        let step_gates = gates.get(index).map_or(&[][..], Vec::as_slice);
        match step_decision(step_gates, before) {
            Decision::Run => {
                *state = StepState::Asked;
                changes.push(Change::Asked(index));
            }
            Decision::Skip => {
                *state = StepState::Skipped;
                changes.push(Change::Skipped(index));
            }
            Decision::Wait => {}
        }
    }
    changes
}
