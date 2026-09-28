//! The job list: every (variant, task, repeat), ordered so a partial run
//! stays comparable, minus jobs a previous run already finished.

use std::path::{Path, PathBuf};

use crate::inputs::{Task, Variant};

/// One run to perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Job<'a> {
    pub variant: &'a Variant,
    pub task: &'a Task,
    /// 1-based.
    pub repeat: u32,
}

impl Job<'_> {
    /// `<out>/runs/<variant>/<task>/<repeat>`.
    #[must_use]
    pub fn dir(&self, out: &Path) -> PathBuf {
        out.join("runs")
            .join(&self.variant.id)
            .join(&self.task.id)
            .join(self.repeat.to_string())
    }

    /// Whether a previous run already recorded this job.
    #[must_use]
    pub fn done(&self, out: &Path) -> bool {
        self.dir(out).join("result.json").is_file()
    }
}

/// Every job, round-robin across variants: repeat-major, then task, then
/// variant, so each prefix of the list covers the variants evenly.
#[must_use]
pub fn plan<'a>(variants: &'a [Variant], tasks: &'a [Task], repeats: u32) -> Vec<Job<'a>> {
    let mut jobs = Vec::with_capacity(
        variants
            .len()
            .saturating_mul(tasks.len())
            .saturating_mul(usize::try_from(repeats).unwrap_or(0)),
    );

    for repeat in 1..=repeats {
        for task in tasks {
            for variant in variants {
                jobs.push(Job {
                    variant,
                    task,
                    repeat,
                });
            }
        }
    }

    jobs
}

/// The jobs without a `result.json` under `out`.
#[must_use]
pub fn pending<'a>(jobs: &[Job<'a>], out: &Path) -> Vec<Job<'a>> {
    jobs.iter().copied().filter(|job| !job.done(out)).collect()
}
