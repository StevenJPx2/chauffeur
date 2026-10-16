//! Threads that call the judges, so the engine thread never waits on one.
//! Judge clients block on HTTPS; each worker owns its own.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

/// A fixed set of worker threads taking tasks from one queue.
pub(crate) struct Pool<T> {
    tasks: Sender<T>,
    size: usize,
    busy: usize,
}

impl<T: Send + 'static> Pool<T> {
    /// `size` threads, each with an executor from `build`, running `work` on
    /// every task and sending what it returns to `done`. Returns once every
    /// executor is built, or with the first error.
    pub(crate) fn spawn<E: 'static, O: Send + 'static>(
        name: &str,
        size: usize,
        build: impl Fn() -> Result<E, String> + Send + Sync + 'static,
        work: impl Fn(&mut E, T) -> O + Send + Sync + 'static,
        done: &Sender<O>,
    ) -> Result<Self, String> {
        let (tasks, queue) = mpsc::channel::<T>();
        let queue = Arc::new(Mutex::new(queue));
        let build = Arc::new(build);
        let work = Arc::new(work);
        let (ready, started) = mpsc::channel();

        for index in 0..size {
            let queue = Arc::clone(&queue);
            let build = Arc::clone(&build);
            let work = Arc::clone(&work);
            let done = done.clone();
            let ready = ready.clone();

            std::thread::Builder::new()
                .name(format!("{name}-{index}"))
                .spawn(move || {
                    let mut executor = match build() {
                        Ok(executor) => {
                            let _ = ready.send(Ok(()));
                            executor
                        }
                        Err(error) => {
                            let _ = ready.send(Err(error));
                            return;
                        }
                    };

                    loop {
                        let task = match queue.lock() {
                            Ok(queue) => queue.recv(),
                            Err(_) => return,
                        };
                        let Ok(task) = task else { return };

                        if done.send(work(&mut executor, task)).is_err() {
                            return;
                        }
                    }
                })
                .map_err(|error| format!("spawn {name}: {error}"))?;
        }

        drop(ready);

        for _ in 0..size {
            started
                .recv()
                .map_err(|_| format!("{name} exited during startup"))??;
        }

        Ok(Self {
            tasks,
            size,
            busy: 0,
        })
    }

    /// Workers not running a task.
    pub(crate) fn free(&self) -> usize {
        self.size.saturating_sub(self.busy)
    }

    /// Hand `task` to a free worker.
    pub(crate) fn send(&mut self, task: T) {
        if self.tasks.send(task).is_ok() {
            self.busy += 1;
        }
    }

    /// A worker finished its task.
    pub(crate) fn finished(&mut self) {
        self.busy = self.busy.saturating_sub(1);
    }
}
