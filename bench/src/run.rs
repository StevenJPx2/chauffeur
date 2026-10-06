//! One run: lay out the repo, start a private daemon for Chauffeur variants,
//! run OpenCode under a deadline, check the result, and record it.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::clock::Utc;
use crate::daemon::{Daemon, DaemonDirs};
use crate::fsutil;
use crate::inputs::Setup;
use crate::jobs::Job;
use crate::process::{ChildGuard, Exit};
use crate::result::RunResult;
use crate::{audit, events, workspace};

/// Programs and settings shared by every run.
#[derive(Clone, Debug)]
pub struct RunConfig {
    pub out: PathBuf,
    pub model: String,
    pub opencode: PathBuf,
    pub chauffeur: PathBuf,
    /// The shipped skills folder each daemon loads: `CHAUFFEUR_SKILLS_DIR`,
    /// else this repo's `skills/`.
    pub skills: PathBuf,
    /// Every variant id in `variants.json` order, recorded per result so the
    /// report keeps that order.
    pub variant_ids: Vec<String>,
}

/// Perform `job` and write its `result.json`. A failure inside the run is
/// recorded in the result, not returned.
///
/// # Errors
/// The run folder or `result.json` cannot be written.
pub fn execute(job: &Job<'_>, config: &RunConfig) -> Result<RunResult, String> {
    let dir = job.dir(&config.out);
    fsutil::remove_dir_if_exists(&dir)?;
    std::fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;
    let mut result = RunResult {
        variant: job.variant.id.clone(),
        task: job.task.id.clone(),
        variant_order: config
            .variant_ids
            .iter()
            .position(|id| *id == job.variant.id)
            .unwrap_or(config.variant_ids.len()),
        repeat: job.repeat,
        model: config.model.clone(),
        started_at: Utc::now().iso(),
        ..RunResult::default()
    };

    if let Err(error) = attempt(job, config, &dir, &mut result) {
        result.passed = false;
        result.error = Some(error);
    }

    let path = dir.join("result.json");
    let text = serde_json::to_string_pretty(&result)
        .map_err(|error| format!("render {}: {error}", path.display()))?;
    std::fs::write(&path, text).map_err(|error| format!("write {}: {error}", path.display()))?;

    Ok(result)
}

fn attempt(
    job: &Job<'_>,
    config: &RunConfig,
    dir: &Path,
    result: &mut RunResult,
) -> Result<(), String> {
    let repo = dir.join("repo");
    let task = job.task;
    workspace::prepare(task, job.variant, &repo)?;
    let env = workspace::task_env(task, &dir.join("bench.log"))?;

    let daemon = match &job.variant.setup {
        Setup::Base => None,
        Setup::Chauffeur { .. } => Some(Daemon::start(
            &config.chauffeur,
            &daemon_dirs(job, config, dir)?,
            &job.variant.disable_csv(),
            &dir.join("daemon.log"),
        )?),
    };
    let agent = run_agent(job, config, dir, &env, daemon.as_ref());
    drop(daemon);
    let (exit, wall) = agent?;

    result.wall_seconds = wall.as_secs_f64();
    result.timed_out = exit == Exit::TimedOut;
    result.exit_code = match exit {
        Exit::Code(code) => code,
        Exit::TimedOut => None,
    };
    result.events = events::parse(&read_optional(&dir.join("events.jsonl"))?);
    if matches!(job.variant.setup, Setup::Chauffeur { .. }) {
        let audit_log = read_optional(&dir.join("state").join("audit.jsonl"))?;
        result.chauffeur = Some(audit::parse(&audit_log));
    }

    workspace::apply_hidden(task, &repo)?;
    let check = workspace::run_check(task, &repo, &env, &dir.join("check.log"))?;
    result.passed = check == Exit::Code(Some(0));

    Ok(())
}

/// The run's own config folder, holding only the variant's overrides, so the
/// user's personal rules and settings stay out of the benchmark.
fn daemon_dirs(job: &Job<'_>, config: &RunConfig, dir: &Path) -> Result<DaemonDirs, String> {
    let config_dir = dir.join("config");

    if let Some(overlay) = &job.variant.config {
        fsutil::overlay(overlay, &config_dir, &[])?;
    }

    Ok(DaemonDirs {
        state: dir.join("state"),
        config: config_dir,
        skills: config.skills.clone(),
    })
}

fn run_agent(
    job: &Job<'_>,
    config: &RunConfig,
    dir: &Path,
    env: &[(OsString, OsString)],
    daemon: Option<&Daemon>,
) -> Result<(Exit, Duration), String> {
    let stdout = create(&dir.join("events.jsonl"))?;
    let stderr = create(&dir.join("stderr.log"))?;
    let mut command = Command::new(&config.opencode);
    // No `--auto`: headless OpenCode rejects every request that would ask the
    // user, so permission decisions count as they would for a real user.
    command
        .args(["run", "--standalone", "--format", "json", "-m"])
        .arg(&config.model)
        .arg(job.task.prompt(&workspace::outside_dir(&dir.join("repo"))))
        .current_dir(dir.join("repo"))
        // OpenCode takes its project from PWD, not the process's directory.
        .env("PWD", dir.join("repo"))
        // A private OPENCODE_DB loses the provider logins, so `-m` falls back
        // to the default model; runs share the user's session database.
        .envs(env.iter().cloned())
        .env_remove("CHAUFFEUR_DAEMON_TOKEN")
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    match daemon {
        Some(daemon) => command
            .env("CHAUFFEUR_DAEMON_URL", daemon.url())
            .env("CHAUFFEUR_DISABLE", job.variant.disable_csv()),
        None => command
            .env_remove("CHAUFFEUR_DAEMON_URL")
            .env_remove("CHAUFFEUR_DISABLE"),
    };
    command.envs(&job.variant.env);

    let started = Instant::now();
    let timeout = Duration::from_secs(job.task.spec.timeout_seconds);
    let exit = ChildGuard::spawn(&mut command, "opencode")?.wait(timeout)?;

    Ok((exit, started.elapsed()))
}

fn create(path: &Path) -> Result<std::fs::File, String> {
    std::fs::File::create(path).map_err(|error| format!("create {}: {error}", path.display()))
}

fn read_optional(path: &Path) -> Result<String, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(format!("read {}: {error}", path.display())),
    }
}
