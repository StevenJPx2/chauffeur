//! Kani harnesses: admission's ledger stays within its capacity, a permit
//! is given back exactly once on every way a navigation ends, and the event
//! log never holds more than its bound.

use crate::admission::{Admission, Capacity, Ledger, Source};
use crate::run::EventLog;

fn any_source() -> Source {
    if kani::any() {
        Source::Harness
    } else {
        Source::Integration
    }
}

fn any_capacity() -> Capacity {
    let global: usize = kani::any();
    let per_source: usize = kani::any();
    kani::assume(global <= 3 && per_source <= 3);
    Capacity { global, per_source }
}

#[kani::proof]
#[kani::unwind(7)]
fn the_ledger_stays_within_its_capacity() {
    let capacity = any_capacity();
    let mut ledger = Ledger::new(capacity);

    for _ in 0..6 {
        let source = any_source();
        if kani::any() {
            let before = ledger.held(source);
            if ledger.take(source).is_ok() {
                assert!(ledger.held(source) == before + 1);
            } else {
                assert!(ledger.held(source) == before);
            }
        } else {
            ledger.give(source);
        }
        let (harness, integration) = (
            ledger.held(Source::Harness),
            ledger.held(Source::Integration),
        );
        assert!(harness <= capacity.per_source && integration <= capacity.per_source);
        assert!(ledger.total() <= capacity.global);
        assert!(ledger.total() == harness + integration);
    }
}

#[kani::proof]
#[kani::unwind(3)]
fn a_permit_is_given_back_once_on_every_path() {
    let admission = Admission::new(Capacity {
        global: 2,
        per_source: 2,
    });
    let source = any_source();
    let kept = admission.admit(source);
    let ending = admission.admit(source);
    assert!(kept.is_ok() && ending.is_ok());

    // Delivered, refused, past its deadline, cancelled or its caller gone:
    // each ends by dropping the permit, directly or with what holds it.
    let path: u8 = kani::any();
    kani::assume(path < 4);
    match path {
        0 => drop(ending),
        1 => {
            let deliver = move || drop(ending);
            deliver();
        }
        2 => {
            let late: Option<_> = ending.ok();
            drop(late);
        }
        _ => {
            let navigations = vec![ending];
            drop(navigations);
        }
    }

    assert!(admission.in_use().held(source) == 1);
    drop(kept);
    assert!(admission.in_use().total() == 0);
}

#[kani::proof]
#[kani::unwind(6)]
fn the_event_log_never_exceeds_its_bound() {
    let max: usize = kani::any();
    kani::assume(max <= 3);
    let mut log = EventLog::new(max);

    for event in 0_u8..5 {
        let kept = log.push(event);
        assert!(log.events().len() <= max);
        assert!(kept || log.events().len() == max);
    }
    assert!(log.events().len() + log.dropped() == 5);
}
