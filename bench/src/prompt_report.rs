//! The report's "First request" section: what each variant's requests are
//! made of, from the probe plugin and the provider's token counts. The first
//! variant is the control; the others show their change against it.

use std::fmt::Write;

use crate::requests::Composition;
use crate::result::RunResult;
use crate::stats::{mean_f64, to_f64};

/// Named system sections or tools listed per part.
const MAX_NAMED: usize = 12;

/// One variant's completed runs that the probe recorded.
struct Column<'a> {
    variant: &'a str,
    firsts: Vec<&'a Composition>,
    prompts: Vec<&'a [u64]>,
}

/// The section, or nothing when no run carried probe records.
#[must_use]
pub fn section(results: &[RunResult], variants: &[&str]) -> String {
    let columns: Vec<Column<'_>> = variants
        .iter()
        .map(|&variant| column(results, variant))
        .filter(|column| !column.firsts.is_empty())
        .collect();

    if columns.is_empty() {
        return String::new();
    }

    let mut table = format!(
        "\n## First request\n\nMeans per run; changes are against {}.\n\n| | {} |\n|---|{}\n",
        columns[0].variant,
        columns
            .iter()
            .map(|c| c.variant)
            .collect::<Vec<_>>()
            .join(" | "),
        "---:|".repeat(columns.len()),
    );

    for (label, step) in [
        ("Prompt tokens, request 1", 0),
        ("Prompt tokens, request 2", 1),
    ] {
        table.push_str(&row(label, &columns, |c| {
            mean(c.prompts.iter().filter_map(|p| p.get(step).copied()))
        }));
    }
    for (label, get) in [
        (
            "Code Mode namespaces",
            (|first| first.namespaces) as fn(&Composition) -> u64,
        ),
        ("Characters sent", |first| first.total_chars),
        ("System", |first| first.system_chars),
        ("Tool definitions", |first| first.tools_chars),
        ("Messages", |first| first.messages_chars),
    ] {
        table.push_str(&row(label, &columns, |c| {
            mean(c.firsts.iter().map(|first| get(first)))
        }));
    }
    for part in ["system", "tools"] {
        for name in named(&columns, part) {
            let label = format!("{part}: {}", name.replace('|', "\\|"));
            table.push_str(&row(&label, &columns, |c| named_mean(c, part, &name)));
        }
    }

    table
}

fn column<'a>(results: &'a [RunResult], variant: &'a str) -> Column<'a> {
    let runs: Vec<&RunResult> = results
        .iter()
        .filter(|run| run.variant == variant && run.error.is_none())
        .collect();

    Column {
        variant,
        firsts: runs
            .iter()
            .filter_map(|run| run.requests.as_ref()?.first.as_ref())
            .collect(),
        prompts: runs
            .iter()
            .map(|run| run.events.step_prompts.as_slice())
            .collect(),
    }
}

fn row(label: &str, columns: &[Column<'_>], value: impl Fn(&Column<'_>) -> f64) -> String {
    let control = value(&columns[0]);
    let mut line = format!("| {label} |");

    for (index, column) in columns.iter().enumerate() {
        let here = value(column);
        let _ = if index == 0 {
            write!(line, " {here:.0} |")
        } else {
            write!(
                line,
                " {here:.0} ({}) |",
                crate::report::change(here, control)
            )
        };
    }
    line.push('\n');

    line
}

fn mean(values: impl Iterator<Item = u64>) -> f64 {
    mean_f64(&values.map(to_f64).collect::<Vec<_>>())
}

/// A named section's mean size; a run without it counts as zero.
fn named_mean(column: &Column<'_>, part: &str, name: &str) -> f64 {
    mean(column.firsts.iter().map(|first| {
        first
            .sections
            .iter()
            .filter(|section| section.part == part && section.name == name)
            .map(|section| section.chars)
            .sum()
    }))
}

/// The largest sections of `part` in any variant, largest first.
fn named(columns: &[Column<'_>], part: &str) -> Vec<String> {
    let mut names: Vec<String> = columns
        .iter()
        .flat_map(|column| column.firsts.iter().flat_map(|first| first.sections.iter()))
        .filter(|section| section.part == part)
        .map(|section| section.name.clone())
        .collect();
    names.sort_unstable();
    names.dedup();

    let largest = |name: &str| {
        columns
            .iter()
            .map(|column| named_mean(column, part, name))
            .fold(0.0_f64, f64::max)
    };
    names.sort_by(|a, b| largest(b).total_cmp(&largest(a)));
    names.truncate(MAX_NAMED);

    names
}
