//! Compare judges by the action Chauffeur would take on each answer.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::{Value, json};

use super::{Case, Outcome};

/// Chauffeur acts on a yes/no answer only when it is this far from even:
/// P ≥ 0.7 or P ≤ 0.3, an effective confidence of 0.4.
const CONFIDENT: f64 = 0.7;

/// What Chauffeur would do with an answer.
#[must_use]
pub fn action(kind: &str, answer: &Value) -> Option<String> {
    match kind {
        "noul" => answer["noul"].as_f64().map(|p| {
            if p >= CONFIDENT {
                "yes".into()
            } else if p <= 1.0 - CONFIDENT {
                "no".into()
            } else {
                "unsure".into()
            }
        }),
        "choice" => answer["choice"].as_str().map(str::to_string),
        "score" => answer["score"]
            .as_f64()
            .map(|score| format!("{}", score.round())),
        _ => None,
    }
}

/// The capability a question belongs to: its ID before the first `/`.
fn capability(id: &str) -> &str {
    id.split_once('/').map_or(id, |(prefix, _)| prefix)
}

#[derive(Default)]
struct Tally {
    questions: u32,
    agree: u32,
    failed_cases: u32,
    latencies: Vec<u64>,
    by_capability: BTreeMap<String, (u32, u32)>,
    labelled: Labelled,
}

/// A judge's answers to labelled disputes: right, confidently wrong, or
/// unsure (no action), out of the total.
#[derive(Clone, Copy, Default)]
struct Labelled {
    right: u32,
    wrong: u32,
    unsure: u32,
    total: u32,
}

impl Labelled {
    fn add(&mut self, theirs: Option<&str>, label: &str) {
        self.total += 1;
        match theirs {
            Some(theirs) if theirs == label => self.right += 1,
            Some("unsure") | None => self.unsure += 1,
            Some(_) => self.wrong += 1,
        }
    }
}

/// Every (case, question) where some judge acts differently from the reference.
#[must_use]
pub fn disputes(cases: &[Case], outcomes: &[Outcome]) -> Vec<Value> {
    let mut found = Vec::new();

    for case in cases {
        for (id, question) in questions(case) {
            let kind = question["type"].as_str().unwrap_or_default();
            let reference = action(kind, &case.reference[id.as_str()]);
            let actions: BTreeMap<&str, Option<String>> = outcomes
                .iter()
                .filter(|outcome| outcome.case == case.id)
                .filter_map(|outcome| {
                    let answers = outcome.answers.as_ref()?;
                    Some((outcome.judge.as_str(), action(kind, &answers[id.as_str()])))
                })
                .collect();

            if actions.values().any(|candidate| *candidate != reference) {
                found.push(json!({
                    "key": format!("{}/{id}", case.id),
                    "instructions": question["instructions"],
                    "criteria": question.get("criteria"),
                    "state": case.request["state"],
                    "reference": reference,
                    "judges": actions,
                }));
            }
        }
    }

    found
}

fn questions(case: &Case) -> Vec<(String, Value)> {
    case.request["questions"]
        .as_object()
        .map(|questions| {
            questions
                .iter()
                .map(|(id, q)| (id.clone(), q.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// The Markdown report. `labels` maps `case/question` to the right action.
#[must_use]
pub fn report(cases: &[Case], outcomes: &[Outcome], labels: &BTreeMap<String, String>) -> String {
    let mut tallies: BTreeMap<&str, Tally> = BTreeMap::new();

    for outcome in outcomes {
        let tally = tallies.entry(outcome.judge.as_str()).or_default();
        let Some(case) = cases.iter().find(|case| case.id == outcome.case) else {
            continue;
        };
        let Some(answers) = &outcome.answers else {
            tally.failed_cases += 1;
            continue;
        };
        tally.latencies.push(outcome.elapsed_ms);

        for (id, question) in questions(case) {
            let kind = question["type"].as_str().unwrap_or_default();
            let theirs = action(kind, &answers[id.as_str()]);
            let agrees = theirs == action(kind, &case.reference[id.as_str()]);
            let entry = tally
                .by_capability
                .entry(capability(&id).to_string())
                .or_default();

            tally.questions += 1;
            tally.agree += u32::from(agrees);
            entry.0 += u32::from(agrees);
            entry.1 += 1;
            if let Some(label) = labels.get(&format!("{}/{id}", case.id)) {
                tally.labelled.add(theirs.as_deref(), label);
            }
        }
    }

    render(
        cases.len(),
        &tallies,
        labels,
        reference_accuracy(cases, labels),
    )
}

/// How the recorded reference itself fares against the labels.
fn reference_accuracy(cases: &[Case], labels: &BTreeMap<String, String>) -> Labelled {
    let mut labelled = Labelled::default();

    for case in cases {
        for (id, question) in questions(case) {
            let Some(label) = labels.get(&format!("{}/{id}", case.id)) else {
                continue;
            };
            let kind = question["type"].as_str().unwrap_or_default();
            labelled.add(action(kind, &case.reference[id.as_str()]).as_deref(), label);
        }
    }

    labelled
}

fn render(
    cases: usize,
    tallies: &BTreeMap<&str, Tally>,
    labels: &BTreeMap<String, String>,
    reference: Labelled,
) -> String {
    let mut out = format!(
        "# Judge benchmark\n\n{cases} recorded judgments, replayed against each judge. \
         \"Same action\" counts questions where the judge's answer leads Chauffeur to the \
         same action as the recorded Jev answer (yes/no bins at P ≥ {CONFIDENT} and \
         ≤ {:.1}; the same choice; the same rounded score).\n\n",
        1.0 - CONFIDENT
    );
    let labelled = !labels.is_empty();

    out.push_str("| Judge | Same action | Failed calls | p50 ms | p95 ms |");
    out.push_str(if labelled {
        " Disputes: right | wrong | unsure |\n|---|---|---|---|---|---|---|---|\n"
    } else {
        "\n|---|---|---|---|---|\n"
    });
    for (judge, tally) in tallies {
        let _ = write!(
            out,
            "| {judge} | {} | {} | {} | {} |",
            percent(tally.agree, tally.questions),
            tally.failed_cases,
            percentile(&tally.latencies, 50),
            percentile(&tally.latencies, 95),
        );
        if labelled {
            out.push_str(&labelled_cells(tally.labelled));
        }
        out.push('\n');
    }
    if labelled {
        let _ = writeln!(
            out,
            "\nOn the {} labelled disputes, the recorded Jev answers were right | wrong | \
             unsure:{}\n\n\"Wrong\" is a confident answer against the label, which Chauffeur \
             acts on; \"unsure\" takes no action.",
            reference.total,
            labelled_cells(reference)
        );
    }

    out.push_str(&by_capability(tallies));
    out
}

fn by_capability(tallies: &BTreeMap<&str, Tally>) -> String {
    let capabilities: Vec<&String> = tallies
        .values()
        .flat_map(|tally| tally.by_capability.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut out = String::from("\n## Same action, by capability\n\n| Judge |");

    for capability in &capabilities {
        let _ = write!(out, " {capability} |");
    }
    let _ = write!(out, "\n|---|{}\n", "---|".repeat(capabilities.len()));
    for (judge, tally) in tallies {
        let _ = write!(out, "| {judge} |");
        for capability in &capabilities {
            let (agree, total) = tally
                .by_capability
                .get(*capability)
                .copied()
                .unwrap_or_default();
            let _ = write!(out, " {} |", percent(agree, total));
        }
        out.push('\n');
    }

    out
}

fn labelled_cells(labelled: Labelled) -> String {
    format!(
        " {} | {} | {} |",
        percent(labelled.right, labelled.total),
        percent(labelled.wrong, labelled.total),
        percent(labelled.unsure, labelled.total)
    )
}

fn percent(part: u32, whole: u32) -> String {
    if whole == 0 {
        return "–".into();
    }

    format!(
        "{:.0}% ({part}/{whole})",
        f64::from(part) * 100.0 / f64::from(whole)
    )
}

fn percentile(values: &[u64], pct: usize) -> String {
    if values.is_empty() {
        return "–".into();
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let index = (sorted.len() * pct).div_ceil(100).saturating_sub(1);

    sorted[index.min(sorted.len() - 1)].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yes_no_answers_bin_at_chauffeurs_confidence() {
        assert_eq!(
            action("noul", &json!({"noul": 0.7})).as_deref(),
            Some("yes")
        );
        assert_eq!(
            action("noul", &json!({"noul": 0.5})).as_deref(),
            Some("unsure")
        );
        assert_eq!(action("noul", &json!({"noul": 0.3})).as_deref(), Some("no"));
        assert_eq!(
            action("choice", &json!({"choice": "b"})).as_deref(),
            Some("b")
        );
        assert_eq!(
            action("score", &json!({"score": 1.4})).as_deref(),
            Some("1")
        );
        assert_eq!(action("noul", &json!({})), None);
    }

    #[test]
    fn labelled_answers_split_into_right_wrong_and_unsure() {
        let mut labelled = Labelled::default();

        labelled.add(Some("no"), "no");
        labelled.add(Some("yes"), "no");
        labelled.add(Some("unsure"), "no");
        labelled.add(None, "yes");

        assert_eq!(
            (
                labelled.right,
                labelled.wrong,
                labelled.unsure,
                labelled.total
            ),
            (1, 1, 2, 4)
        );
    }

    #[test]
    fn percentiles_pick_the_nearest_rank() {
        assert_eq!(percentile(&[30, 10, 20, 40], 50), "20");
        assert_eq!(percentile(&[30, 10, 20, 40], 95), "40");
        assert_eq!(percentile(&[], 50), "–");
    }
}
