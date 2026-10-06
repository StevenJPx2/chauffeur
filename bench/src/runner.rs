//! The `run` command: plan the jobs, run the pending ones on a bounded pool of
//! worker threads, append each result as it lands, and write the report.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::clock::Utc;
use crate::inputs::{self, Setup, Task, Variant};
use crate::jobs::{self, Job};
use crate::result::RunResult;
use crate::run::{self, RunConfig};
use crate::{report, which};

/// Options for `chauffeur-bench run`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOptions {
    pub tasks: Option<Vec<String>>,
    pub variants: Option<Vec<String>>,
    pub repeats: u32,
    pub parallel: usize,
    pub model: String,
    pub out: Option<PathBuf>,
    pub chauffeur: String,
    pub opencode: String,
}

/// Run every pending job and write `report.md`.
///
/// # Errors
/// Invalid inputs, a missing program, or a result that cannot be recorded.
pub fn run(root: &Path, options: &RunOptions) -> Result<(), String> {
    let all_variants = inputs::load_variants(root)?;
    let variants = inputs::select(
        &all_variants,
        options.variants.as_deref(),
        |variant: &Variant| &variant.id,
        "variant",
    )?;
    let tasks = inputs::select(
        &inputs::load_tasks(root)?,
        options.tasks.as_deref(),
        |task: &Task| &task.id,
        "task",
    )?;
    let config = config(options, &variants, &all_variants)?;
    let all = jobs::plan(&variants, &tasks, options.repeats);
    let pending = jobs::pending(&all, &config.out);
    eprintln!(
        "chauffeur-bench: {} jobs, {} already done, results in {}",
        all.len(),
        all.len().saturating_sub(pending.len()),
        config.out.display()
    );

    let outcome = run_pool(&pending, &config, options.parallel);
    let results = config.out.join("results.jsonl");
    if results.is_file() {
        let path = report::write(&config.out)?;
        eprintln!("chauffeur-bench: report at {}", path.display());
    }

    outcome
}

fn config(
    options: &RunOptions,
    variants: &[Variant],
    all_variants: &[Variant],
) -> Result<RunConfig, String> {
    let skills = skills_dir()?;
    let opencode = which::resolve(&options.opencode)?;
    let needs_chauffeur = variants
        .iter()
        .any(|variant| matches!(variant.setup, Setup::Chauffeur { .. }));
    let chauffeur = if needs_chauffeur {
        which::resolve(&options.chauffeur)?
    } else {
        PathBuf::from(&options.chauffeur)
    };
    let out = match &options.out {
        Some(out) => out.clone(),
        None => default_out()?.join(Utc::now().compact()),
    };
    std::fs::create_dir_all(&out).map_err(|error| format!("create {}: {error}", out.display()))?;
    let out = out
        .canonicalize()
        .map_err(|error| format!("resolve {}: {error}", out.display()))?;
    outside_repos(&out)?;

    Ok(RunConfig {
        out,
        model: options.model.clone(),
        opencode,
        chauffeur,
        skills,
        variant_ids: all_variants
            .iter()
            .map(|variant| variant.id.clone())
            .collect(),
    })
}

/// `CHAUFFEUR_SKILLS_DIR`, else the `skills/` folder of the checkout this
/// harness was built from: the shipped skills under test.
///
/// # Errors
/// The folder does not exist.
pub fn skills_dir() -> Result<PathBuf, String> {
    let skills = std::env::var_os("CHAUFFEUR_SKILLS_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../skills"),
        PathBuf::from,
    );

    if skills.is_dir() {
        Ok(skills)
    } else {
        Err(format!(
            "no skills folder at {}; set CHAUFFEUR_SKILLS_DIR",
            skills.display()
        ))
    }
}

/// `$XDG_STATE_HOME/chauffeur/bench`, else `~/.local/state/chauffeur/bench`.
fn default_out() -> Result<PathBuf, String> {
    let state = match std::env::var_os("XDG_STATE_HOME") {
        Some(state) => PathBuf::from(state),
        None => {
            PathBuf::from(std::env::var_os("HOME").ok_or("HOME is not set")?).join(".local/state")
        }
    };

    Ok(state.join("chauffeur/bench"))
}

/// Refuse an output folder inside a git checkout: the agent would see the
/// checkout (such as this repo's `bench/tasks/`) and could edit it instead.
pub fn outside_repos(out: &Path) -> Result<(), String> {
    match out.ancestors().find(|dir| dir.join(".git").exists()) {
        Some(repo) => Err(format!(
            "--out {} is inside the git checkout {}; choose a folder outside any repo",
            out.display(),
            repo.display()
        )),
        None => Ok(()),
    }
}

/// Workers pull jobs in order; a failure to record a result stops new jobs.
fn run_pool(jobs: &[Job<'_>], config: &RunConfig, parallel: usize) -> Result<(), String> {
    let next = AtomicUsize::new(0);
    let finished = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let failures = Mutex::new(Vec::new());
    let sink = Mutex::new(());

    std::thread::scope(|scope| {
        for _ in 0..parallel.clamp(1, jobs.len().max(1)) {
            scope.spawn(|| {
                while !stop.load(Ordering::SeqCst) {
                    let Some(job) = jobs.get(next.fetch_add(1, Ordering::SeqCst)) else {
                        break;
                    };
                    let outcome = run::execute(job, config)
                        .and_then(|result| append(&config.out, &sink, &result).map(|()| result));
                    let done = finished.fetch_add(1, Ordering::SeqCst).saturating_add(1);
                    match outcome {
                        Ok(result) => eprintln!("[{done}/{}] {}", jobs.len(), progress(&result)),
                        Err(error) => {
                            stop.store(true, Ordering::SeqCst);
                            eprintln!("[{done}/{}] stopping: {error}", jobs.len());
                            if let Ok(mut failures) = failures.lock() {
                                failures.push(error);
                            }
                        }
                    }
                }
            });
        }
    });

    let failures = failures.into_inner().unwrap_or_default();
    if failures.is_empty() {
        return Ok(());
    }

    Err(failures.join("; "))
}

fn append(out: &Path, sink: &Mutex<()>, result: &RunResult) -> Result<(), String> {
    let path = out.join("results.jsonl");
    let line = serde_json::to_string(result).map_err(|error| format!("render result: {error}"))?;
    let _held = sink
        .lock()
        .map_err(|_| "results.jsonl lock poisoned".to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| format!("open {}: {error}", path.display()))?;

    writeln!(file, "{line}").map_err(|error| format!("append {}: {error}", path.display()))
}

fn progress(result: &RunResult) -> String {
    let verdict = match (&result.error, result.passed, result.timed_out) {
        (Some(error), _, _) => format!("error: {error}"),
        (None, true, _) => "pass".into(),
        (None, false, true) => "fail (timed out)".into(),
        (None, false, false) => "fail".into(),
    };

    format!(
        "{} {} #{}: {verdict} ({:.1}s, ${:.4})",
        result.variant, result.task, result.repeat, result.wall_seconds, result.events.cost
    )
}
