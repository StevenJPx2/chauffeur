//! The `verify` command: prove each task's check fails on the starting repo
//! and passes with its solution, without a model.

use std::path::Path;

use crate::inputs::{Setup, Task};
use crate::process::Exit;
use crate::workspace;

/// What a check did on one copy of a task.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Passed,
    Failed(Exit),
    /// The copy could not be prepared or checked.
    Broken(String),
    /// The task has no `solution/`.
    NoSolution,
}

/// One task's verification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub task: String,
    pub unsolved: Outcome,
    pub solved: Outcome,
}

impl Verdict {
    /// Unsolved must fail; solved must pass, when a solution exists.
    #[must_use]
    pub fn ok(&self) -> bool {
        matches!(self.unsolved, Outcome::Failed(_))
            && matches!(self.solved, Outcome::Passed | Outcome::NoSolution)
    }
}

/// Verify every task, with working copies under `scratch`.
#[must_use]
pub fn verify(tasks: &[Task], scratch: &Path) -> Vec<Verdict> {
    tasks
        .iter()
        .map(|task| {
            let dir = scratch.join(&task.id);
            let solved = if task.solution().is_some() {
                check_copy(task, &dir.join("solved"), true)
            } else {
                Outcome::NoSolution
            };

            Verdict {
                task: task.id.clone(),
                unsolved: check_copy(task, &dir.join("unsolved"), false),
                solved,
            }
        })
        .collect()
}

/// Lay out a copy as a run would (solution standing in for the agent, then
/// hidden), and run the check.
fn check_copy(task: &Task, dir: &Path, solve: bool) -> Outcome {
    let attempt = || -> Result<Exit, String> {
        crate::fsutil::remove_dir_if_exists(dir)?;
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("create {}: {error}", dir.display()))?;
        let repo = dir.join("repo");
        workspace::prepare(task, &Setup::Base, &repo)?;
        let env = workspace::task_env(task, &dir.join("bench.log"))?;
        if solve {
            workspace::apply_solution(task, &repo, &env, &dir.join("solve.log"))?;
        }
        workspace::apply_hidden(task, &repo)?;

        workspace::run_check(task, &repo, &env, &dir.join("check.log"))
    };

    match attempt() {
        Ok(Exit::Code(Some(0))) => Outcome::Passed,
        Ok(exit) => Outcome::Failed(exit),
        Err(error) => Outcome::Broken(error),
    }
}

/// A Markdown table of verdicts.
#[must_use]
pub fn render(verdicts: &[Verdict]) -> String {
    let mut table =
        String::from("| Task | Without solution | With solution | OK |\n|---|---|---|---|\n");

    for verdict in verdicts {
        table.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            verdict.task,
            describe(&verdict.unsolved),
            describe(&verdict.solved),
            if verdict.ok() { "yes" } else { "NO" }
        ));
    }

    table
}

fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Passed => "pass".into(),
        Outcome::Failed(Exit::Code(Some(code))) => format!("fail (exit {code})"),
        Outcome::Failed(Exit::Code(None)) => "fail (signal)".into(),
        Outcome::Failed(Exit::TimedOut) => "fail (timeout)".into(),
        Outcome::Broken(error) => format!("error: {}", error.replace('|', "/")),
        Outcome::NoSolution => "no solution".into(),
    }
}
