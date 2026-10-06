//! `report.md`, built only from `results.jsonl`.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use crate::result::{self, RunResult};
use crate::stats::{mean_f64, min_max, to_f64};

const BASE: &str = "base";

/// Rebuild `<dir>/report.md` from `<dir>/results.jsonl`.
///
/// # Errors
/// An unreadable results file or an unwritable report.
pub fn write(dir: &Path) -> Result<PathBuf, String> {
    let results = result::read_jsonl(&dir.join("results.jsonl"))?;
    let path = dir.join("report.md");
    std::fs::write(&path, render(&results))
        .map_err(|error| format!("write {}: {error}", path.display()))?;

    Ok(path)
}

/// The whole report.
#[must_use]
pub fn render(results: &[RunResult]) -> String {
    let variants = variant_order(results);
    let summaries: Vec<(&str, Summary)> = variants
        .iter()
        .map(|&variant| (variant, Summary::of(runs_of(results, variant))))
        .collect();
    let mut report = header(results, &variants);

    report.push_str(&summary_table(&summaries));
    report.push_str(&pass_matrix(results, &variants));
    report.push_str(&activity_table(results, &variants));
    report.push_str(&versus_base(&summaries));
    report.push_str(&crate::prompt_report::section(results, &variants));

    report
}

/// Variants in `variants.json` order, ties by first appearance.
fn variant_order(results: &[RunResult]) -> Vec<&str> {
    let mut order: Vec<(usize, &str)> = Vec::new();

    for result in results {
        if !order.iter().any(|(_, id)| *id == result.variant) {
            order.push((result.variant_order, &result.variant));
        }
    }
    order.sort_by_key(|(rank, _)| *rank);

    order.into_iter().map(|(_, id)| id).collect()
}

fn runs_of<'a>(results: &'a [RunResult], variant: &str) -> Vec<&'a RunResult> {
    results
        .iter()
        .filter(|result| result.variant == variant)
        .collect()
}

fn header(results: &[RunResult], variants: &[&str]) -> String {
    let mut models: Vec<&str> = results.iter().map(|r| r.model.as_str()).collect();
    models.sort_unstable();
    models.dedup();
    let tasks = task_order(results);
    let repeats = results.iter().map(|r| r.repeat).max().unwrap_or(0);
    let first = results.iter().map(|r| r.started_at.as_str()).min();
    let last = results.iter().map(|r| r.started_at.as_str()).max();
    let date = match (first, last) {
        (Some(first), Some(last)) if first != last => format!("{first} – {last}"),
        (Some(first), _) => first.to_string(),
        _ => "–".into(),
    };

    format!(
        "# Chauffeur bench report\n\n\
         | | |\n|---|---|\n\
         | Model | {} |\n| Repeats | {repeats} |\n| Tasks | {} ({}) |\n\
         | Variants | {} |\n| Runs | {} |\n| Date | {date} |\n",
        models.join(", "),
        tasks.join(", "),
        tasks.len(),
        variants.join(", "),
        results.len(),
    )
}

fn task_order(results: &[RunResult]) -> Vec<&str> {
    let mut tasks: Vec<&str> = results.iter().map(|r| r.task.as_str()).collect();
    tasks.sort_unstable();
    tasks.dedup();

    tasks
}

/// One variant's outcome and cost figures. Runs with a harness error count
/// only toward `runs` and `errors`.
struct Summary {
    runs: usize,
    completed: usize,
    passed: usize,
    input: Vec<f64>,
    output: Vec<f64>,
    cache_read: Vec<f64>,
    cost: Vec<f64>,
    wall: Vec<f64>,
    steps: Vec<f64>,
    tool_calls: Vec<f64>,
    tool_errors: Vec<f64>,
    timeouts: usize,
    errors: usize,
}

impl Summary {
    fn of(all: Vec<&RunResult>) -> Self {
        let runs: Vec<&RunResult> = all
            .iter()
            .copied()
            .filter(|run| run.error.is_none())
            .collect();
        let field = |get: fn(&RunResult) -> u64| -> Vec<f64> {
            runs.iter().map(|run| to_f64(get(run))).collect()
        };

        Self {
            runs: all.len(),
            completed: runs.len(),
            passed: runs.iter().filter(|run| run.passed).count(),
            input: field(|run| run.events.tokens.input),
            output: field(|run| run.events.tokens.output),
            cache_read: field(|run| run.events.tokens.cache_read),
            cost: runs.iter().map(|run| run.events.cost).collect(),
            wall: runs.iter().map(|run| run.wall_seconds).collect(),
            steps: field(|run| run.events.steps),
            tool_calls: field(|run| run.events.tool_calls),
            tool_errors: field(|run| run.events.tool_errors),
            timeouts: runs.iter().filter(|run| run.timed_out).count(),
            errors: all.len().saturating_sub(runs.len()),
        }
    }

    fn pass_rate(&self) -> f64 {
        ratio(self.passed, self.completed)
    }
}

fn ratio(part: usize, whole: usize) -> f64 {
    let as_f64 = |n: usize| to_f64(u64::try_from(n).unwrap_or(u64::MAX));

    if whole == 0 {
        return 0.0;
    }

    as_f64(part) / as_f64(whole)
}

fn summary_table(summaries: &[(&str, Summary)]) -> String {
    let mut table = String::from(
        "\n## Summary\n\n\
         | Variant | Runs | Pass rate | Input tokens | Output tokens | Cache read | Cost | Wall s | Steps | Tool calls | Tool errors | Timeouts | Harness errors |\n\
         |---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );

    for (variant, s) in summaries {
        let _ = writeln!(
            table,
            "| {variant} | {} | {:.1}% ({}/{}) | {} | {} | {} | ${:.4} | {} | {:.1} | {:.1} | {:.1} | {} | {} |",
            s.runs,
            s.pass_rate() * 100.0,
            s.passed,
            s.completed,
            with_range(&s.input, 0),
            with_range(&s.output, 0),
            with_range(&s.cache_read, 0),
            mean_f64(&s.cost),
            with_range(&s.wall, 1),
            mean_f64(&s.steps),
            mean_f64(&s.tool_calls),
            mean_f64(&s.tool_errors),
            s.timeouts,
            s.errors,
        );
    }
    table.push_str(
        "\nPass rates and means cover completed runs; a harness error (the run \
         could not finish, e.g. its daemon never became healthy) is counted \
         only under Harness errors. Ranges are min–max.\n",
    );

    table
}

/// `mean (min–max)` with `digits` decimals.
fn with_range(values: &[f64], digits: usize) -> String {
    match min_max(values) {
        Some((low, high)) => format!(
            "{:.digits$} ({low:.digits$}–{high:.digits$})",
            mean_f64(values)
        ),
        None => "–".into(),
    }
}

fn pass_matrix(results: &[RunResult], variants: &[&str]) -> String {
    let mut table = format!(
        "\n## Pass matrix\n\n| Task | {} |\n|---|{}\n",
        variants.join(" | "),
        "---:|".repeat(variants.len())
    );

    for task in task_order(results) {
        let cells: Vec<String> = variants
            .iter()
            .map(|&variant| {
                pass_cell(
                    results
                        .iter()
                        .filter(|r| r.task == task && r.variant == variant),
                )
            })
            .collect();
        let _ = writeln!(table, "| {task} | {} |", cells.join(" | "));
    }

    table
}

/// `passed/completed`, noting harness errors; `–` for no runs.
fn pass_cell<'a>(runs: impl Iterator<Item = &'a RunResult>) -> String {
    let (mut passed, mut completed, mut errors) = (0_usize, 0_usize, 0_usize);

    for run in runs {
        match (&run.error, run.passed) {
            (Some(_), _) => errors = errors.saturating_add(1),
            (None, true) => {
                passed = passed.saturating_add(1);
                completed = completed.saturating_add(1);
            }
            (None, false) => completed = completed.saturating_add(1),
        }
    }

    match (completed, errors) {
        (0, 0) => "–".into(),
        (_, 0) => format!("{passed}/{completed}"),
        _ => format!("{passed}/{completed} (+{errors} err)"),
    }
}

fn activity_table(results: &[RunResult], variants: &[&str]) -> String {
    let mut rows = String::new();

    for &variant in variants {
        let metrics: Vec<_> = runs_of(results, variant)
            .into_iter()
            .filter_map(|run| run.chauffeur.as_ref())
            .collect();
        if metrics.is_empty() {
            continue;
        }

        let runs = metrics.len();
        let per_run = |get: fn(&crate::audit::ChauffeurMetrics) -> u64| {
            let total: u64 = metrics.iter().map(|m| get(m)).sum();
            ratio(usize::try_from(total).unwrap_or(usize::MAX), runs)
        };
        let calls: u64 = metrics.iter().map(|m| m.jev_calls).sum();
        let elapsed: u64 = metrics.iter().map(|m| m.jev_elapsed_ms_total).sum();
        let mut skills = BTreeMap::new();
        let mut permissions = BTreeMap::new();
        for m in &metrics {
            merge(&mut skills, &m.skills);
            merge(&mut permissions, &m.permissions);
        }
        let _ = writeln!(
            rows,
            "| {variant} | {runs} | {:.1} | {:.0} | {} | {:.1} | {:.1} | {:.1} | {} | {} |",
            per_run(|m| m.jev_calls),
            crate::stats::mean(elapsed, calls),
            metrics.iter().map(|m| m.jev_errors).sum::<u64>(),
            per_run(|m| m.tools_hidden),
            per_run(|m| m.tools_revealed),
            per_run(|m| m.steers),
            counts(&permissions),
            counts(&skills),
        );
    }

    if rows.is_empty() {
        return String::new();
    }

    format!(
        "\n## Chauffeur activity\n\n\
         | Variant | Runs | Jev calls/run | Mean Jev latency ms | Jev errors | Tools hidden/run | Tools revealed/run | Steers/run | Permissions | Skills attached |\n\
         |---|---:|---:|---:|---:|---:|---:|---:|---|---|\n{rows}"
    )
}

fn merge(into: &mut BTreeMap<String, u64>, from: &BTreeMap<String, u64>) {
    for (key, count) in from {
        let entry = into.entry(key.clone()).or_insert(0);
        *entry = entry.saturating_add(*count);
    }
}

/// `a ×3, b ×1`, most frequent first.
fn counts(map: &BTreeMap<String, u64>) -> String {
    if map.is_empty() {
        return "–".into();
    }

    let mut entries: Vec<(&String, &u64)> = map.iter().collect();
    entries.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));

    entries
        .iter()
        .map(|(key, count)| format!("{key} ×{count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn versus_base(summaries: &[(&str, Summary)]) -> String {
    let Some((_, base)) = summaries.iter().find(|(variant, _)| *variant == BASE) else {
        return String::new();
    };
    let mut table = String::from(
        "\n## vs base\n\n\
         | Variant | Pass rate Δ | Input tokens | Cost | Wall time |\n|---|---:|---:|---:|---:|\n",
    );

    for (variant, s) in summaries.iter().filter(|(variant, _)| *variant != BASE) {
        if s.completed == 0 || base.completed == 0 {
            let _ = writeln!(table, "| {variant} | n/a | n/a | n/a | n/a |");
            continue;
        }

        let _ = writeln!(
            table,
            "| {variant} | {:+.1} pp | {} | {} | {} |",
            (s.pass_rate() - base.pass_rate()) * 100.0,
            change(mean_f64(&s.input), mean_f64(&base.input)),
            change(mean_f64(&s.cost), mean_f64(&base.cost)),
            change(mean_f64(&s.wall), mean_f64(&base.wall)),
        );
    }

    table
}

/// Percentage change from `base` to `value`.
pub(crate) fn change(value: f64, base: f64) -> String {
    if base.abs() < f64::EPSILON {
        return "n/a".into();
    }

    format!("{:+.1}%", (value - base) / base * 100.0)
}
