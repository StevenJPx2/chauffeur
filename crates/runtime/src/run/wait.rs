//! Waiting for a job on a worker until it finishes, its deadline passes or
//! its caller cancels it. A result that comes after is dropped.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use super::pool::Pool;

/// How often a waiting caller looks at its cancellation.
const LOOK: Duration = Duration::from_millis(10);

/// A navigation's cancellation, shared by its caller and its runs. Once
/// cancelled it stays cancelled.
#[derive(Clone, Debug, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A cancellation not yet made.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancels every run that shares it.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether it was cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// How waiting for a job ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Waited<T> {
    /// The job finished in time.
    Done(T),
    /// The deadline passed first.
    Deadline,
    /// The caller cancelled first.
    Cancelled,
    /// The job could not start, or its worker stopped without a result.
    Refused(String),
}

/// Runs `job` on `pool` and waits for it until `deadline` or until
/// `cancel`. The job is given the deadline and the cancellation, to check
/// before each round it starts.
pub fn within<T: Send + 'static>(
    pool: &Pool,
    deadline: Instant,
    cancel: &Cancel,
    job: impl FnOnce(Instant, &Cancel) -> T + Send + 'static,
) -> Waited<T> {
    if cancel.is_cancelled() {
        return Waited::Cancelled;
    }
    let (send, receive) = mpsc::sync_channel(1);
    let shared = cancel.clone();
    // A send after the caller stopped waiting fails, and the result is dropped.
    let started = pool.execute(Box::new(move || {
        let _ = send.send(job(deadline, &shared));
    }));
    if let Err(error) = started {
        return Waited::Refused(format!("cannot start a worker: {error}"));
    }
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Waited::Deadline;
        }
        let received = receive.recv_timeout(left.min(LOOK));
        // A caller that cancelled has gone; whatever came for it is dropped.
        if cancel.is_cancelled() {
            return Waited::Cancelled;
        }
        match received {
            Ok(done) => return Waited::Done(done),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Waited::Refused("the run's worker stopped".to_owned());
            }
        }
    }
}
