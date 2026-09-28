use std::collections::BTreeMap;

use chauffeur_bench::events::Tokens;
use chauffeur_bench::{audit, events};

fn counts(pairs: &[(&str, u64)]) -> BTreeMap<String, u64> {
    pairs
        .iter()
        .map(|(key, count)| ((*key).to_string(), *count))
        .collect()
}

#[test]
fn events_fixture_yields_tokens_cost_tools_and_errors() {
    let metrics = events::parse(include_str!("fixtures/events.jsonl"));

    assert_eq!(metrics.events, 14);
    assert_eq!(metrics.bad_lines, 2);
    assert_eq!(metrics.steps, 2);
    assert_eq!(
        metrics.tokens,
        Tokens {
            input: 2000,
            output: 200,
            reasoning: 10,
            cache_read: 1300,
            cache_write: 50,
        }
    );
    assert!((metrics.cost - 0.02).abs() < 1e-9);
    assert_eq!(
        metrics.tool_calls, 3,
        "duplicate and running parts are not calls"
    );
    assert_eq!(metrics.tools, counts(&[("bash", 2), ("read", 1)]));
    assert_eq!(metrics.tool_errors, 1);
    assert_eq!(metrics.error_events, 2);
    assert_eq!(metrics.first_error.as_deref(), Some("rate limited"));
    assert_eq!(
        metrics.last_text.as_deref(),
        Some("Fixed the failing test.")
    );
}

#[test]
fn last_text_is_clipped_to_500_chars() {
    let long = "é".repeat(700);
    let line = format!(r#"{{"type":"text","part":{{"text":"{long}"}}}}"#);

    let metrics = events::parse(&line);

    assert_eq!(
        metrics.last_text.map(|text| text.chars().count()),
        Some(500)
    );
}

#[test]
fn empty_output_is_all_zero() {
    assert_eq!(events::parse(""), events::EventMetrics::default());
}

#[test]
fn audit_fixture_yields_chauffeur_activity() {
    let metrics = audit::parse(include_str!("fixtures/audit.jsonl"));

    assert_eq!(metrics.records, 6);
    assert_eq!(metrics.bad_lines, 1);
    assert_eq!(
        metrics.signals,
        counts(&[
            ("agent_request", 1),
            ("permission_request", 2),
            ("tool_result", 1),
            ("turn_end", 1),
            ("user_message", 1),
        ])
    );
    assert_eq!(
        metrics.jev_calls, 4,
        "vetoed and quiet traces did not ask Jev"
    );
    assert_eq!(metrics.jev_elapsed_ms_total, 5900);
    assert!((metrics.jev_elapsed_ms_mean - 1475.0).abs() < 1e-9);
    assert_eq!(metrics.jev_errors, 1);
    assert_eq!(metrics.record_errors, 1);
    assert_eq!(
        metrics.skills,
        counts(&[("code-discipline", 1), ("rust-skills", 2)])
    );
    assert_eq!(metrics.tools_hidden, 3);
    assert_eq!(metrics.tools_revealed, 1);
    assert_eq!(metrics.steers, 1);
    assert_eq!(metrics.permissions, counts(&[("allow", 1), ("deny", 1)]));
}
