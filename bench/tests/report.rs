use std::collections::BTreeMap;

use chauffeur_bench::audit::ChauffeurMetrics;
use chauffeur_bench::events::{EventMetrics, Tokens};
use chauffeur_bench::report;
use chauffeur_bench::result::{self, RunResult};

fn run(variant: &str, task: &str, passed: bool, input: u64, cost: f64, wall: f64) -> RunResult {
    RunResult {
        variant: variant.into(),
        task: task.into(),
        repeat: 1,
        model: "openai/gpt-6-luna".into(),
        started_at: format!("2026-09-28T10:00:{:02}Z", input / 100),
        passed,
        wall_seconds: wall,
        events: EventMetrics {
            steps: 4,
            tokens: Tokens {
                input,
                output: input / 10,
                cache_read: input * 2,
                ..Tokens::default()
            },
            cost,
            tool_calls: 6,
            tool_errors: 1,
            ..EventMetrics::default()
        },
        ..RunResult::default()
    }
}

fn activity(jev_calls: u64, elapsed: u64, steers: u64) -> ChauffeurMetrics {
    ChauffeurMetrics {
        jev_calls,
        jev_elapsed_ms_total: elapsed,
        steers,
        skills: BTreeMap::from([("rust-skills".to_string(), 1)]),
        permissions: BTreeMap::from([("allow".to_string(), steers)]),
        tools_hidden: 3,
        ..ChauffeurMetrics::default()
    }
}

fn fixture() -> Vec<RunResult> {
    let mut timed_out = run("base", "b", false, 3000, 0.30, 30.0);
    timed_out.timed_out = true;
    let mut full_a = run("full", "a", true, 800, 0.08, 12.0);
    full_a.chauffeur = Some(activity(2, 600, 0));
    full_a.variant_order = 1;
    let mut full_b = run("full", "b", true, 1600, 0.12, 18.0);
    full_b.chauffeur = Some(activity(4, 1400, 1));
    full_b.variant_order = 1;
    let mut harness_error = run("full", "b", false, 0, 0.0, 0.0);
    harness_error.repeat = 2;
    harness_error.variant_order = 1;
    harness_error.error = Some("daemon never became healthy".into());

    // Completion order, as a parallel run appends them.
    vec![
        full_a,
        run("base", "a", true, 1000, 0.10, 10.0),
        harness_error,
        timed_out,
        full_b,
    ]
}

#[test]
fn renders_summary_matrix_activity_and_deltas() {
    let text = report::render(&fixture());

    assert!(text.contains("| Model | openai/gpt-6-luna |"), "{text}");
    assert!(text.contains("| Tasks | a, b (2) |"), "{text}");
    assert!(text.contains("| Variants | base, full |"), "{text}");
    assert!(text.contains("| Runs | 5 |"), "{text}");
    assert!(text.contains("| Date | 2026-09-28T10:00:00Z – 2026-09-28T10:00:30Z |"));
    assert!(
        text.contains(
            "| base | 2 | 50.0% (1/2) | 2000 (1000–3000) | 200 (100–300) | 4000 (2000–6000) \
             | $0.2000 | 20.0 (10.0–30.0) | 4.0 | 6.0 | 1.0 | 1 | 0 |"
        ),
        "{text}"
    );
    assert!(
        text.contains("| full | 3 | 100.0% (2/2) | 1200 (800–1600) |"),
        "{text}"
    );
    assert!(text.contains("| 0 | 1 |\n"), "full's harness error: {text}");
    assert!(text.contains("| Task | base | full |"), "{text}");
    assert!(text.contains("| a | 1/1 | 1/1 |"), "{text}");
    assert!(text.contains("| b | 0/1 | 1/1 (+1 err) |"), "{text}");
    assert!(
        text.contains("| full | 2 | 3.0 | 333 | 0 | 3.0 | 0.0 | 0.5 | allow ×1 | rust-skills ×2 |"),
        "{text}"
    );
    assert!(
        !text.contains("| base | 2 | 3.0"),
        "base has no activity row"
    );
    assert!(
        text.contains("| full | +50.0 pp | -40.0% | -50.0% | -25.0% |"),
        "{text}"
    );
}

#[test]
fn omits_vs_base_without_a_base_variant() {
    let results: Vec<RunResult> = fixture()
        .into_iter()
        .filter(|result| result.variant != "base")
        .collect();
    let text = report::render(&results);

    assert!(!text.contains("## vs base"));
    assert!(text.contains("## Chauffeur activity"));
}

#[test]
fn later_results_replace_earlier_ones_for_the_same_job() {
    let first = run("base", "a", false, 1000, 0.1, 1.0);
    let second = run("base", "a", true, 1000, 0.1, 1.0);
    let other = run("full", "a", true, 1000, 0.1, 1.0);
    let text = [&first, &other, &second]
        .iter()
        .map(|result| serde_json::to_string(result).unwrap())
        .collect::<Vec<_>>()
        .join("\n");

    let parsed = result::parse_jsonl(&text).unwrap();

    assert_eq!(parsed, vec![second, other]);
    assert!(result::parse_jsonl("{\"nope\":1}").is_err());
}
