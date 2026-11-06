//! Worker threads kept between runs, so a run does not start a thread.

use std::io;
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex, Weak};
use std::thread;

/// Work handed to a worker.
pub type Job = Box<dyn FnOnce() + Send>;

/// Worker threads that wait for another job once theirs is done.
///
/// A job goes to a waiting worker, or to a new one when none waits, so a
/// slow job never holds up the next. At most `keep` workers wait; the others
/// end, and so do the waiting ones when the pool is dropped.
pub struct Pool {
    idle: Arc<Idle>,
}

/// The workers waiting for a job, each by the sender that hands it one.
struct Idle {
    workers: Mutex<Vec<SyncSender<Job>>>,
    keep: usize,
}

impl Pool {
    /// A pool that keeps at most `keep` workers waiting.
    #[must_use]
    pub fn new(keep: usize) -> Self {
        Self {
            idle: Arc::new(Idle {
                workers: Mutex::new(Vec::new()),
                keep,
            }),
        }
    }

    /// Runs `job` on a waiting worker, or on a new one.
    ///
    /// # Errors
    ///
    /// When no worker waits and a new thread cannot start.
    pub fn execute(&self, job: Job) -> io::Result<()> {
        let waiting = self
            .idle
            .workers
            .lock()
            .ok()
            .and_then(|mut workers| workers.pop());
        let Some(worker) = waiting else {
            return self.start(job);
        };
        // A waiting worker is always receiving; should it be gone, start one.
        worker
            .send(job)
            .or_else(|mpsc::SendError(job)| self.start(job))
    }

    /// How many workers wait for a job.
    #[must_use]
    pub fn waiting(&self) -> usize {
        self.idle.workers.lock().map_or(0, |workers| workers.len())
    }

    fn start(&self, job: Job) -> io::Result<()> {
        let idle = Arc::downgrade(&self.idle);
        thread::Builder::new()
            .name("chauffeur worker".to_owned())
            .spawn(move || work(&idle, job))
            .map(drop)
    }
}

/// Runs jobs until the worker is not kept. Holding the pool only weakly,
/// a waiting worker's sender goes with the pool, which ends its wait.
fn work(idle: &Weak<Idle>, first: Job) {
    let mut job = first;
    loop {
        job();
        let (send, receive) = mpsc::sync_channel(1);
        if let Some(idle) = idle.upgrade() {
            idle.wait(send);
        }
        // A worker that was not kept has dropped its sender: no job comes.
        let Ok(next) = receive.recv() else {
            return;
        };
        job = next;
    }
}

impl Idle {
    /// Adds a worker to the waiting ones, unless `keep` already wait.
    fn wait(&self, worker: SyncSender<Job>) {
        if let Ok(mut workers) = self.workers.lock()
            && workers.len() < self.keep
        {
            workers.push(worker);
        }
    }
}
