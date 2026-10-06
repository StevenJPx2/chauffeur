//! The record of one run, written to `result.json` and `results.jsonl`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::audit::ChauffeurMetrics;
use crate::events::EventMetrics;
use crate::requests::RequestMetrics;

/// Everything measured for one (variant, task, repeat).
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RunResult {
    pub variant: String,
    /// The variant's position in `variants.json`; orders report columns.
    pub variant_order: usize,
    pub task: String,
    pub repeat: u32,
    pub model: String,
    /// UTC, `YYYY-MM-DDTHH:MM:SSZ`.
    pub started_at: String,
    pub passed: bool,
    pub timed_out: bool,
    /// OpenCode's exit code; `None` when it was killed or never ran.
    pub exit_code: Option<i32>,
    /// OpenCode's wall-clock time.
    pub wall_seconds: f64,
    /// Why the harness could not complete the run, if it could not.
    pub error: Option<String>,
    pub events: EventMetrics,
    /// What the requests were made of, from the probe plugin.
    #[serde(default)]
    pub requests: Option<RequestMetrics>,
    /// Present for Chauffeur variants.
    pub chauffeur: Option<ChauffeurMetrics>,
}

/// Read every result in a `results.jsonl`. When a job appears more than once
/// the last record wins.
///
/// # Errors
/// An unreadable file or a line that is not a result.
pub fn read_jsonl(path: &Path) -> Result<Vec<RunResult>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;

    parse_jsonl(&text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Parse `results.jsonl` text; see [`read_jsonl`].
///
/// # Errors
/// A non-empty line that is not a result.
pub fn parse_jsonl(text: &str) -> Result<Vec<RunResult>, String> {
    let mut results: Vec<RunResult> = Vec::new();

    for (index, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }

        let result: RunResult = serde_json::from_str(line)
            .map_err(|error| format!("line {}: {error}", index.saturating_add(1)))?;
        let earlier = results.iter_mut().find(|kept| {
            (kept.variant.as_str(), kept.task.as_str(), kept.repeat)
                == (result.variant.as_str(), result.task.as_str(), result.repeat)
        });
        match earlier {
            Some(kept) => *kept = result,
            None => results.push(result),
        }
    }

    Ok(results)
}
