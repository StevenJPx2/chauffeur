#![expect(
    clippy::unwrap_used,
    reason = "a job that never reports fails the test"
)]

use std::sync::mpsc::{self, Receiver};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

use chauffeur_runtime::Pool;

const WAIT: Duration = Duration::from_secs(2);

/// Whether `done` holds within `WAIT`: a worker joins the waiting ones just
/// after its job reports.
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

/// Runs a job on `pool` that reports the thread it ran on.
fn thread_of(pool: &Pool) -> ThreadId {
    let (send, receive) = mpsc::channel();
    pool.execute(Box::new(move || send.send(thread::current().id()).unwrap()))
        .unwrap();
    receive.recv_timeout(WAIT).unwrap()
}

/// Runs a job on `pool` that waits for `release`, then reports to `done`.
fn held(pool: &Pool, release: Receiver<()>, done: mpsc::Sender<()>) {
    pool.execute(Box::new(move || {
        release.recv().unwrap();
        done.send(()).unwrap();
    }))
    .unwrap();
}

#[test]
fn a_finished_worker_takes_the_next_job() {
    let pool = Pool::new(1, 8);

    let first = thread_of(&pool);
    assert!(eventually(|| pool.waiting() == 1));
    let second = thread_of(&pool);

    assert_eq!(first, second);
    assert_ne!(first, thread::current().id());
}

#[test]
fn a_busy_worker_does_not_hold_up_the_next_job() {
    let pool = Pool::new(1, 8);
    let (release, wait) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    held(&pool, wait, done);

    let other = thread_of(&pool);

    assert_ne!(other, thread::current().id());
    release.send(()).unwrap();
    finished.recv_timeout(WAIT).unwrap();
}

#[test]
fn at_most_keep_workers_wait() {
    let pool = Pool::new(2, 8);
    let (done, finished) = mpsc::channel();
    let releases: Vec<_> = (0..3)
        .map(|_| {
            let (release, wait) = mpsc::channel();
            held(&pool, wait, done.clone());
            release
        })
        .collect();

    for release in releases {
        release.send(()).unwrap();
        finished.recv_timeout(WAIT).unwrap();
    }

    assert!(eventually(|| pool.waiting() == 2));
    thread::sleep(Duration::from_millis(50));
    assert_eq!(pool.waiting(), 2);
}
