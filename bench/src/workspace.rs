//! A run's working copy of a task: laid out, configured for the variant,
//! committed, and checked.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::fsutil;
use crate::inputs::{Setup, Task};
use crate::process::{self, Exit};

/// Upper bound on one check (or `.solve.sh`) run.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(600);
const GIT_TIMEOUT: Duration = Duration::from_secs(60);
const SOLVE_SCRIPT: &str = ".solve.sh";

/// The plugin list for `.opencode/opencode.jsonc`. Chauffeur loads from the
/// user's global plugin, so base disables it by name.
#[must_use]
pub fn plugin_list(setup: &Setup) -> Vec<&'static str> {
    match setup {
        Setup::Base => vec!["-chauffeur", "-ntfy-notify", "-sourcefed"],
        Setup::Chauffeur { .. } => vec!["-ntfy-notify", "-sourcefed"],
    }
}

/// The project config text written into the repo copy.
///
/// # Errors
/// Never in practice; serialization of a string list cannot fail.
pub fn opencode_config(setup: &Setup) -> Result<String, String> {
    let config = serde_json::json!({ "plugins": plugin_list(setup) });

    serde_json::to_string_pretty(&config).map_err(|error| format!("render config: {error}"))
}

/// Copy the task's repo to `dest`, write the variant's OpenCode config, and
/// commit it all in a fresh git repository.
///
/// # Errors
/// A failed copy, write, or git command.
pub fn prepare(task: &Task, setup: &Setup, dest: &Path) -> Result<(), String> {
    fsutil::overlay(&task.repo(), dest, &[])?;
    let config_dir = dest.join(".opencode");
    std::fs::create_dir_all(&config_dir)
        .map_err(|error| format!("create {}: {error}", config_dir.display()))?;
    let config = config_dir.join("opencode.jsonc");
    std::fs::write(&config, opencode_config(setup)?)
        .map_err(|error| format!("write {}: {error}", config.display()))?;

    for args in [
        &["init", "-q"][..],
        &["config", "user.name", "chauffeur-bench"],
        &["config", "user.email", "bench@chauffeur.invalid"],
        &["config", "commit.gpgsign", "false"],
        &["add", "-A"],
        &["commit", "-q", "--no-verify", "-m", "task"],
    ] {
        git(dest, args)?;
    }

    Ok(())
}

fn git(repo: &Path, args: &[&str]) -> Result<(), String> {
    let what = format!("git {}", args.join(" "));
    let exit = process::run(
        Command::new("git")
            .args(args)
            .current_dir(repo)
            .stdin(Stdio::null())
            .stdout(Stdio::null()),
        &what,
        GIT_TIMEOUT,
    )?;

    match exit {
        Exit::Code(Some(0)) => Ok(()),
        other => Err(format!("{what} in {}: {other:?}", repo.display())),
    }
}

/// `PATH` (with the task's stubs first) and `BENCH_LOG`, shared by the agent,
/// `.solve.sh`, and the check.
///
/// # Errors
/// A `PATH` that cannot be rebuilt with the stub folder.
pub fn task_env(task: &Task, bench_log: &Path) -> Result<Vec<(OsString, OsString)>, String> {
    let mut env = vec![("BENCH_LOG".into(), bench_log.as_os_str().to_os_string())];

    if let Some(bin) = task.bin() {
        let inherited = std::env::var_os("PATH").unwrap_or_default();
        let mut paths: Vec<PathBuf> = vec![bin];
        paths.extend(std::env::split_paths(&inherited));
        let joined = std::env::join_paths(paths).map_err(|error| format!("build PATH: {error}"))?;
        env.push(("PATH".into(), joined));
    }

    Ok(env)
}

/// Copy the task's `hidden/` over the repo, if it has one.
///
/// # Errors
/// A failed copy.
pub fn apply_hidden(task: &Task, repo: &Path) -> Result<(), String> {
    task.hidden()
        .map_or(Ok(()), |hidden| fsutil::overlay(&hidden, repo, &[]))
}

/// Copy the task's `solution/` over the repo and run its `.solve.sh`.
///
/// # Errors
/// A task without a solution, a failed copy, or a failing script.
pub fn apply_solution(
    task: &Task,
    repo: &Path,
    env: &[(OsString, OsString)],
    log: &Path,
) -> Result<(), String> {
    let solution = task
        .solution()
        .ok_or_else(|| format!("task {} has no solution/", task.id))?;
    fsutil::overlay(&solution, repo, &[SOLVE_SCRIPT])?;
    let script = solution.join(SOLVE_SCRIPT);
    if !script.is_file() {
        return Ok(());
    }

    let mut command = Command::new("sh");
    command
        .arg(&script)
        .current_dir(repo)
        .envs(env.iter().cloned());
    match run_logged(&mut command, log, ".solve.sh")? {
        Exit::Code(Some(0)) => Ok(()),
        other => Err(format!("{} failed: {other:?}", script.display())),
    }
}

/// Run the task's check in `repo`, output to `log`.
///
/// # Errors
/// The check cannot be started or the log cannot be written.
pub fn run_check(
    task: &Task,
    repo: &Path,
    env: &[(OsString, OsString)],
    log: &Path,
) -> Result<Exit, String> {
    let (program, args) = task
        .spec
        .check
        .split_first()
        .ok_or_else(|| format!("task {} has an empty check", task.id))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(repo)
        .envs(env.iter().cloned());

    run_logged(&mut command, log, "check")
}

fn run_logged(command: &mut Command, log: &Path, what: &str) -> Result<Exit, String> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|error| format!("open {}: {error}", log.display()))?;
    let copy = file
        .try_clone()
        .map_err(|error| format!("open {}: {error}", log.display()))?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::from(file))
        .stderr(Stdio::from(copy));

    process::run(command, what, CHECK_TIMEOUT)
}
