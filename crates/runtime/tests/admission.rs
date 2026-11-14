//! Admission, waiting with a deadline and cancellation, the bounded pool and
//! the event log. No file or network is touched, so Miri runs these too.
#![expect(
    clippy::unwrap_used,
    reason = "a step that cannot happen fails the test"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use chauffeur_runtime::{
    Admission, Busy, Cancel, Capacity, EventLog, Pool, Source, Waited, within,
};

const WAIT: Duration = Duration::from_secs(5);

fn soon(milliseconds: u64) -> Instant {
    Instant::now()
        .checked_add(Duration::from_millis(milliseconds))
        .unwrap()
}

/// Whether `done` holds within `WAIT`.
fn eventually(done: impl Fn() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < WAIT {
        if done() {
            return true;
        }
        thread::sleep(Duration::from_millis(1));
    }
    done()
}

#[test]
fn a_flood_of_81_with_capacity_8_admits_8_and_refuses_73() {
    let admission = Admission::new(Capacity {
        global: 8,
        per_source: 8,
    });

    let (admitted, refused): (Vec<_>, Vec<_>) = (0..81)
        .map(|_| admission.admit(Source::Integration))
        .partition(Result::is_ok);

    assert_eq!((admitted.len(), refused.len()), (8, 73));
    assert!(refused.iter().all(|busy| matches!(
        busy,
        Err(Busy {
            from: Source::Integration,
            share: true
        })
    )));
    drop(admitted);
    assert_eq!(admission.in_use().total(), 0);
    assert!(admission.admit(Source::Integration).is_ok());
}

#[test]
fn one_source_at_its_share_leaves_room_for_another() {
    let admission = Admission::new(Capacity::DAEMON);

    let _harness: Vec<_> = (0..8)
        .map(|_| admission.admit(Source::Harness).unwrap())
        .collect();

    assert_eq!(
        admission.admit(Source::Harness).unwrap_err(),
        Busy {
            from: Source::Harness,
            share: true
        }
    );
    let integration = admission.admit(Source::Integration).unwrap();
    assert_eq!(integration.source(), Source::Integration);
    assert_eq!(admission.in_use().held(Source::Harness), 8);
}

#[test]
fn a_full_capacity_refuses_every_source() {
    let admission = Admission::new(Capacity {
        global: 2,
        per_source: 2,
    });

    let _held = [
        admission.admit(Source::Harness).unwrap(),
        admission.admit(Source::Harness).unwrap(),
    ];

    assert_eq!(
        admission.admit(Source::Integration).unwrap_err(),
        Busy {
            from: Source::Integration,
            share: false
        }
    );
}

#[test]
fn a_permit_is_given_back_when_its_thread_panics() {
    let admission = Admission::new(Capacity {
        global: 1,
        per_source: 1,
    });
    let permit = admission.admit(Source::Harness).unwrap();

    let ended = thread::spawn(move || {
        let _held = permit;
        panic!("the navigation's thread fails");
    })
    .join();

    assert!(ended.is_err());
    assert_eq!(admission.in_use().total(), 0);
}

#[test]
fn a_job_that_finishes_in_time_is_done() {
    let pool = Pool::new(1, 4);

    let waited = within(&pool, soon(2_000), &Cancel::new(), |_, _| 42);

    assert_eq!(waited, Waited::Done(42));
}

#[test]
fn a_job_past_its_deadline_is_dropped_and_its_worker_freed() {
    let pool = Pool::new(1, 4);
    let finished = Arc::new(AtomicBool::new(false));
    let late = Arc::clone(&finished);

    let started = Instant::now();
    let waited = within(&pool, soon(20), &Cancel::new(), move |_, _| {
        thread::sleep(Duration::from_millis(150));
        late.store(true, Ordering::SeqCst);
        "late"
    });

    assert_eq!(waited, Waited::Deadline);
    assert!(started.elapsed() < Duration::from_millis(150));
    assert!(eventually(|| finished.load(Ordering::SeqCst)));
    assert!(eventually(|| pool.waiting() == 1));
}

#[test]
fn cancelling_stops_the_wait_and_the_job_sees_it() {
    let pool = Pool::new(1, 4);
    let cancel = Cancel::new();
    let saw = Arc::new(AtomicBool::new(false));
    let seen = Arc::clone(&saw);
    let canceller = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        canceller.cancel();
    });

    let waited = within(&pool, soon(4_000), &cancel, move |deadline, cancel| {
        while Instant::now() < deadline && !cancel.is_cancelled() {
            thread::sleep(Duration::from_millis(1));
        }
        seen.store(cancel.is_cancelled(), Ordering::SeqCst);
    });

    assert_eq!(waited, Waited::Cancelled);
    assert!(eventually(|| saw.load(Ordering::SeqCst)));
}

#[test]
fn a_job_cancelled_before_it_starts_never_runs() {
    let pool = Pool::new(1, 4);
    let cancel = Cancel::new();
    cancel.cancel();
    let ran = Arc::new(AtomicBool::new(false));
    let running = Arc::clone(&ran);

    let waited = within(&pool, soon(1_000), &cancel, move |_, _| {
        running.store(true, Ordering::SeqCst);
    });

    assert_eq!(waited, Waited::Cancelled);
    assert!(!ran.load(Ordering::SeqCst));
    assert_eq!(pool.alive(), 0);
}

#[test]
fn past_its_most_workers_the_pool_refuses_a_job() {
    let pool = Pool::new(0, 1);
    let release = Arc::new(AtomicBool::new(false));
    let held = Arc::clone(&release);
    let first = Cancel::new();
    let holding = thread::scope(|scope| {
        let busy = scope.spawn(|| {
            within(&pool, soon(2_000), &first, move |_, _| {
                while !held.load(Ordering::SeqCst) {
                    thread::sleep(Duration::from_millis(1));
                }
            })
        });
        assert!(eventually(|| pool.alive() == 1));

        let refused = within(&pool, soon(1_000), &Cancel::new(), |_, _| ());

        release.store(true, Ordering::SeqCst);
        (refused, busy.join().unwrap())
    });

    assert!(
        matches!(&holding.0, Waited::Refused(reason) if reason.contains("busy")),
        "{holding:?}"
    );
    assert_eq!(holding.1, Waited::Done(()));
    assert!(eventually(|| pool.alive() == 0));
}

#[test]
fn the_event_log_keeps_at_most_its_bound_and_counts_the_rest() {
    let mut log = EventLog::new(3);

    let kept: Vec<bool> = (0..5).map(|event| log.push(event)).collect();

    assert_eq!(kept, [true, true, true, false, false]);
    assert_eq!(log.events(), [0, 1, 2]);
    assert_eq!(log.dropped(), 2);
}
