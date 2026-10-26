//! Kani proofs over the gate kernel. Each harness proves one property for
//! every combination of states, gates and answers within its bounds; run them
//! one at a time with `cargo kani -p chauffeur-core --harness <name>`.

use crate::gate::{Change, Decision, Link, StepState, decide, resolve, step_decision};
use crate::plan::{Mode, Outcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum A {
    Yes,
    No,
}

fn any_outcome() -> Outcome<A> {
    match kani::any::<u8>() {
        0 => Outcome::Value(A::Yes),
        1 => Outcome::Value(A::No),
        _ => Outcome::Failed,
    }
}

fn any_state() -> StepState<A> {
    match kani::any::<u8>() {
        0 => StepState::Pending,
        1 => StepState::Asked,
        2 => StepState::Skipped,
        _ => StepState::Settled(any_outcome()),
    }
}

fn any_terminal() -> StepState<A> {
    if kani::any() {
        StepState::Skipped
    } else {
        StepState::Settled(any_outcome())
    }
}

fn any_mode() -> Mode {
    if kani::any() { Mode::Any } else { Mode::All }
}

/// A non-empty subset of the possible outcomes, as validation requires.
/// Vectors are pre-sized throughout so Kani need not model reallocation.
fn any_on() -> Vec<Outcome<A>> {
    let mut on = Vec::with_capacity(3);
    for outcome in [
        Outcome::Value(A::Yes),
        Outcome::Value(A::No),
        Outcome::Failed,
    ] {
        if kani::any() {
            on.push(outcome);
        }
    }
    kani::assume(!on.is_empty());
    on
}

fn matched(state: &StepState<A>, on: &[Outcome<A>]) -> bool {
    matches!(state, StepState::Settled(outcome) if on.contains(outcome))
}

#[kani::proof]
#[kani::unwind(4)]
fn any_gate_runs_on_a_match_and_skips_when_none_remains_possible() {
    let sources = [any_state(), any_state(), any_state()];
    let on = any_on();

    let decision = decide(Mode::Any, &on, &sources);

    let some_match = sources.iter().any(|state| matched(state, &on));
    let all_terminal = sources.iter().all(StepState::is_terminal);
    assert_eq!(decision == Decision::Run, some_match);
    assert_eq!(decision == Decision::Skip, !some_match && all_terminal);
}

#[kani::proof]
#[kani::unwind(4)]
fn all_gate_runs_when_every_source_matches_and_skips_on_a_terminal_mismatch() {
    let sources = [any_state(), any_state(), any_state()];
    let on = any_on();

    let decision = decide(Mode::All, &on, &sources);

    let every_match = sources.iter().all(|state| matched(state, &on));
    let terminal_mismatch = sources
        .iter()
        .any(|state| state.is_terminal() && !matched(state, &on));
    assert_eq!(decision == Decision::Run, every_match);
    assert_eq!(decision == Decision::Skip, terminal_mismatch);
}

/// Short-circuiting is sound: once a gate decides, no later answer to a
/// still-open source can change the decision.
#[kani::proof]
#[kani::unwind(4)]
fn a_gate_decision_is_final() {
    let mode = any_mode();
    let on = any_on();
    let mut sources = [any_state(), any_state(), any_state()];
    let before = decide(mode, &on, &sources);
    kani::assume(before != Decision::Wait);
    let index: usize = kani::any();
    kani::assume(index < sources.len());
    kani::assume(!sources[index].is_terminal());

    sources[index] = any_terminal();

    assert_eq!(decide(mode, &on, &sources), before);
}

/// A gate on `step`: a non-empty subset of the steps before it.
fn any_link(step: usize) -> Link<A> {
    let mut sources = Vec::with_capacity(2);
    for source in 0..step {
        if kani::any() {
            sources.push(source);
        }
    }
    kani::assume(!sources.is_empty());
    Link {
        mode: any_mode(),
        sources,
        on: any_on(),
    }
}

/// A plan of three steps, each ungated or gated on one earlier step settling
/// to one outcome. Gates over several sources and outcomes, and several gates
/// on one step, are proven separately above and below; this shape keeps every
/// vector a fixed length so the whole-plan proofs stay small.
fn any_plan() -> Vec<Vec<Link<A>>> {
    let mut plan = Vec::with_capacity(3);
    for step in 0..3_usize {
        let mut gates = Vec::with_capacity(1);
        if step > 0 && kani::any() {
            let source: usize = kani::any();
            kani::assume(source < step);
            gates.push(Link {
                mode: any_mode(),
                sources: vec![source],
                on: vec![any_outcome()],
            });
        }
        plan.push(gates);
    }
    plan
}

/// A step's gates combine with AND: one skip skips it, and it runs only when
/// every gate runs.
#[kani::proof]
#[kani::unwind(4)]
fn a_step_runs_only_when_every_gate_runs() {
    let before = [any_state(), any_state()];
    let gates = [any_link(2), any_link(2)];

    let decision = step_decision(&gates, &before);

    let alone = |gate: &Link<A>| {
        decide(
            gate.mode,
            &gate.on,
            gate.sources.iter().map(|&s| &before[s]),
        )
    };
    let each = [alone(&gates[0]), alone(&gates[1])];
    assert_eq!(decision == Decision::Skip, each.contains(&Decision::Skip));
    assert_eq!(
        decision == Decision::Run,
        each.iter().all(|d| *d == Decision::Run)
    );
}

/// Skips propagate within one pass: a second pass over its result changes
/// nothing, whatever state the steps started in.
#[kani::proof]
#[kani::unwind(4)]
fn one_pass_settles_every_decidable_step() {
    let plan = any_plan();
    let mut states = [any_state(), any_state(), any_state()];

    let _ = resolve(&plan, &mut states);
    let after = states.clone();

    assert!(resolve(&plan, &mut states).is_empty());
    assert!(states == after);
}

// Termination follows from the two proofs below. A pass changes only pending
// steps, and only to asked or skipped; answering turns asked into settled. So
// a step leaves "pending" at most once and is asked at most once. A pass with
// no open round either asks a step, removing it from "pending" for good, or
// finds every step terminal. Hence a plan of n steps finishes within n rounds.

/// A pass only moves pending steps forward, and reports exactly what it moved.
#[kani::proof]
#[kani::unwind(4)]
fn a_pass_only_moves_pending_steps_forward() {
    let plan = any_plan();
    let before = [any_state(), any_state(), any_state()];
    let mut after = before.clone();

    let changes = resolve(&plan, &mut after);

    let mut asked = [false; 3];
    let mut skipped = [false; 3];
    for change in changes {
        match change {
            Change::Asked(step) => asked[step] = true,
            Change::Skipped(step) => skipped[step] = true,
        }
    }
    for step in 0..3 {
        let (asked, skipped) = (asked[step], skipped[step]);
        match (&before[step], &after[step]) {
            (StepState::Pending, StepState::Pending) => assert!(!asked && !skipped),
            (StepState::Pending, StepState::Asked) => assert!(asked && !skipped),
            (StepState::Pending, StepState::Skipped) => assert!(skipped && !asked),
            (was, now) => assert!(was == now && !asked && !skipped),
        }
    }
}

/// With no round open, a pass never stalls: it asks a step or finishes.
#[kani::proof]
#[kani::unwind(4)]
fn a_pass_with_no_open_round_asks_or_finishes() {
    let plan = any_plan();
    let mut states = [any_state(), any_state(), any_state()];
    kani::assume(states.iter().all(|state| *state != StepState::Asked));

    let _ = resolve(&plan, &mut states);

    // A pass marks the steps it asks as asked, as the proof above shows.
    let asked = states.iter().any(|state| *state == StepState::Asked);
    assert!(asked || states.iter().all(StepState::is_terminal));
}
