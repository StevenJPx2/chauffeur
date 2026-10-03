//! Replay recorded System One judgments against other decision models.
//!
//! A daemon run with `CHAUFFEUR_RECORD_JUDGMENTS=FILE` appends each request it
//! sent Jev and the answers it got. Every judge here speaks the same
//! `/v1/systemone` format, so each recorded request replays unchanged, with
//! only the model swapped.

pub mod call;
pub mod score;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use call::{Caller, Judge};

/// `chauffeur-bench judges` options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JudgeOptions {
    pub corpus: PathBuf,
    pub judges: PathBuf,
    pub only: Option<Vec<String>>,
    pub out: PathBuf,
    pub limit: Option<usize>,
}

/// One recorded judgment: what was asked and what Jev answered.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Case {
    pub id: usize,
    pub request: Value,
    pub reference: Map<String, Value>,
}

/// One judge's answers to one case.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Outcome {
    pub judge: String,
    pub case: usize,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answers: Option<Map<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Replay the corpus, then write `cases.jsonl`, `results.jsonl`,
/// `disputes.jsonl` and `report.md` to the output folder.
///
/// # Errors
/// An unreadable corpus or judge list, a judge missing its credentials, or an
/// unwritable output folder.
pub fn run(options: &JudgeOptions) -> Result<PathBuf, String> {
    let cases = load_corpus(&options.corpus, options.limit)?;
    let judges = load_judges(&options.judges, options.only.as_deref())?;
    let callers = judges
        .iter()
        .map(|judge| Caller::new(judge).map(|caller| (judge.name.clone(), caller)))
        .collect::<Result<Vec<_>, _>>()?;

    fs::create_dir_all(&options.out)
        .map_err(|error| format!("create {}: {error}", options.out.display()))?;
    eprintln!(
        "replaying {} judgments against {} judges",
        cases.len(),
        callers.len()
    );

    let outcomes: Vec<Outcome> = std::thread::scope(|scope| {
        let runs: Vec<_> = callers
            .iter()
            .map(|(name, caller)| scope.spawn(|| replay(name, caller, &cases)))
            .collect();

        runs.into_iter()
            .flat_map(|run| run.join().unwrap_or_default())
            .collect()
    });

    write_jsonl(&options.out.join("cases.jsonl"), &cases)?;
    write_jsonl(&options.out.join("results.jsonl"), &outcomes)?;
    report(&options.out, None)
}

/// Rebuild `disputes.jsonl` and `report.md` in `dir`, scoring against
/// `labels` (a JSON object of `case/question` to the right action) if given.
///
/// # Errors
/// Missing or unreadable files in `dir`, or an unreadable labels file.
pub fn report(dir: &Path, labels: Option<&Path>) -> Result<PathBuf, String> {
    let cases: Vec<Case> = read_jsonl(&dir.join("cases.jsonl"))?;
    let outcomes: Vec<Outcome> = read_jsonl(&dir.join("results.jsonl"))?;
    let labels: BTreeMap<String, String> = match labels {
        Some(path) => serde_json::from_str(&read(path)?)
            .map_err(|error| format!("parse {}: {error}", path.display()))?,
        None => BTreeMap::new(),
    };

    write_jsonl(
        &dir.join("disputes.jsonl"),
        &score::disputes(&cases, &outcomes),
    )?;
    let path = dir.join("report.md");
    fs::write(&path, score::report(&cases, &outcomes, &labels))
        .map_err(|error| format!("write {}: {error}", path.display()))?;

    Ok(path)
}

fn replay(name: &str, caller: &Caller, cases: &[Case]) -> Vec<Outcome> {
    // The first call can load a local model; keep it out of the timings.
    if let Some(case) = cases.first() {
        let _ = caller.ask(&case.request);
    }

    cases
        .iter()
        .map(|case| {
            let started = Instant::now();
            let result = caller.ask(&case.request);
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let (answers, error) = match result {
                Ok(answers) => (Some(answers), None),
                Err(error) => (None, Some(error)),
            };

            Outcome {
                judge: name.to_string(),
                case: case.id,
                elapsed_ms,
                answers,
                error,
            }
        })
        .collect()
}

/// Recorded judgments that Jev answered; failed calls have no reference.
fn load_corpus(path: &Path, limit: Option<usize>) -> Result<Vec<Case>, String> {
    let text = read(path)?;
    let cases: Vec<Case> = text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|mut line| {
            let reference = line["reply"]["answers"].as_object()?.clone();
            Some((line["request"].take(), reference))
        })
        .take(limit.unwrap_or(usize::MAX))
        .enumerate()
        .map(|(id, (request, reference))| Case {
            id,
            request,
            reference,
        })
        .collect();

    if cases.is_empty() {
        return Err(format!("{} holds no answered judgments", path.display()));
    }

    Ok(cases)
}

fn load_judges(path: &Path, only: Option<&[String]>) -> Result<Vec<Judge>, String> {
    let judges: Vec<Judge> = serde_json::from_str(&read(path)?)
        .map_err(|error| format!("parse {}: {error}", path.display()))?;
    let Some(only) = only else { return Ok(judges) };

    if let Some(unknown) = only
        .iter()
        .find(|name| !judges.iter().any(|judge| judge.name == **name))
    {
        return Err(format!("{} has no judge {unknown}", path.display()));
    }

    Ok(judges
        .into_iter()
        .filter(|judge| only.contains(&judge.name))
        .collect())
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))
}

fn read_jsonl<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Vec<T>, String> {
    read(path)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).map_err(|error| format!("parse {}: {error}", path.display()))
        })
        .collect()
}

fn write_jsonl<T: Serialize>(path: &Path, items: &[T]) -> Result<(), String> {
    let text: String = items
        .iter()
        .filter_map(|item| serde_json::to_string(item).ok())
        .map(|line| line + "\n")
        .collect();

    fs::write(path, text).map_err(|error| format!("write {}: {error}", path.display()))
}
