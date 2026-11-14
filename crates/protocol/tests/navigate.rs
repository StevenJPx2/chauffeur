//! The `navigate` fixtures every binding replays, and the wire shapes of
//! intents, rejections and a run's events.
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "the helpers fail the test on a broken fixture"
)]

use std::path::PathBuf;

use chauffeur_core::{Failure, Outcome, Panel};
use chauffeur_judges::{Answer, AnsweredBy, Confidence, Milli, Probabilities, Question};
use chauffeur_protocol::{
    DEFAULT_DEADLINE, Intent, Limit, Navigate, Rejection, Rule, Target, progress,
};
use serde_json::{Value, json};

/// The fixture cases, each with its `fill` applied: `text` repeated
/// `times` as the named block of its `evidence` or `candidates`.
fn cases() -> Vec<Value> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/navigate.json");
    let fixtures: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut cases = fixtures["cases"].as_array().unwrap().clone();
    for case in &mut cases {
        for fill in case["fill"].as_array().cloned().unwrap_or_default() {
            let (into, name) = (fill["in"].as_str().unwrap(), fill["name"].as_str().unwrap());
            let times = usize::try_from(fill["times"].as_u64().unwrap()).unwrap();
            case["params"][into][name] = json!(fill["text"].as_str().unwrap().repeat(times));
        }
    }
    cases
}

#[test]
fn every_fixture_is_accepted_or_rejected_as_it_expects() {
    let cases = cases();
    assert_eq!(cases.len(), 51);

    for case in cases {
        let name = case["name"].as_str().unwrap();
        let parsed = Navigate::parse(&case["params"]);
        match (&case["expect"], &case["reject"], parsed) {
            (Value::String(expect), Value::Null, Ok(navigate)) => {
                let kind = match navigate.target() {
                    Target::On(_) => "on",
                    Target::Strategy(_) => "strategy",
                };
                assert_eq!(kind, expect, "{name}");
            }
            (Value::Null, reject, Err(rejection)) => {
                let error = rejection.to_error();
                assert_eq!(error["data"]["reason"], reject["reason"], "{name}: {error}");
                if let Some(limit) = reject.get("limit") {
                    assert_eq!(&error["data"]["limit"], limit, "{name}: {error}");
                }
            }
            (_, _, outcome) => panic!("{name}: {outcome:?}"),
        }
    }
}

#[test]
fn every_limit_has_a_fixture_over_it() {
    let rejected: Vec<Value> = cases()
        .iter()
        .filter_map(|case| case["reject"].get("limit").cloned())
        .collect();

    for limit in Limit::ALL {
        assert!(rejected.contains(&json!(limit.name())), "{}", limit.name());
    }
}

#[test]
fn a_rejection_names_its_reason_and_limit() {
    let params = json!({ "protocol": 1, "on": "x", "evidence": { "log": "a".repeat(262_145) } });

    let error = Navigate::parse(&params).unwrap_err().to_error();

    assert_eq!(
        error,
        json!({ "code": -32602, "message": "evidence is over 256 KiB",
                "data": { "reason": "limit_exceeded", "limit": "evidence_bytes", "max": 262_144 } })
    );
    let protocol = Navigate::parse(&json!({ "protocol": 9, "on": "x" })).unwrap_err();
    assert_eq!(protocol.to_error()["data"]["accepted"], json!([1]));
    assert_eq!(Rejection::Busy.to_error()["code"], -32000);
}

#[test]
fn an_event_request_matches_rules_by_its_on_and_evidence() {
    let navigate = Navigate::parse(&json!({
        "protocol": 1, "on": "sourcefed.event", "evidence": { "event": "CI failed" }
    }))
    .unwrap();
    let rule = Rule::parse(
        "sourcefed/triage",
        r#"{ "on": "sourcefed.event", "strategy": [{ "id": "screen", "judge": "system-one",
             "ask": { "outcomes": ["yes", "no"], "evidence": ["event"], "instructions": "Show it?" } }],
             "then": [{ "if": { "screen": "no" }, "withhold": "event" }] }"#,
    )
    .unwrap();

    let event = navigate.event().unwrap();

    assert!(rule.hooks(&event));
    assert_eq!(event.field("event"), Some(&json!("CI failed")));
    assert_eq!(navigate.deadline(), DEFAULT_DEADLINE);
    assert_eq!(
        navigate.evidence(),
        [("event".to_owned(), "CI failed".to_owned())]
    );
    assert_eq!(navigate.candidates(), []);

    let recall = Navigate::parse(&json!({
        "protocol": 1, "on": "opencode-history.recall", "candidates": { "p4": "…", "p1": "…" }
    }))
    .unwrap();
    let names: Vec<&str> = recall
        .candidates()
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(names, ["p1", "p4"], "in name order");
}

#[test]
fn an_evidence_block_named_event_does_not_rename_the_event() {
    let navigate = Navigate::parse(&json!({
        "protocol": 1, "on": "x", "evidence": { "event": "y" }
    }))
    .unwrap();

    let event = navigate.event().unwrap();

    assert_eq!(event.name(), "x");
    assert_eq!(event.evidence(), [("event".to_owned(), "y".to_owned())]);
}

#[test]
fn an_intent_carries_its_rule_and_reads_back() {
    let rule = Rule::parse(
        "sourcefed/triage",
        r#"{ "on": "x", "strategy": [{ "id": "s", "judge": "system-one",
             "ask": { "outcomes": ["yes", "no"], "instructions": "?" } }],
             "then": [{ "if": { "s": "no" }, "withhold": "event" }] }"#,
    )
    .unwrap();

    let intent = Intent::new(rule.id(), &rule.then()[0]);
    let wire = intent.to_json();

    assert_eq!(
        (intent.rule(), intent.name()),
        ("sourcefed/triage", "withhold")
    );
    assert_eq!(intent.payload(), &json!("event"));

    assert_eq!(
        wire,
        json!({ "rule": "sourcefed/triage", "withhold": "event" })
    );
    assert_eq!(Intent::from_json(&wire), Some(intent));
    assert_eq!(Intent::from_json(&json!({ "rule": "r" })), None);
    assert_eq!(
        Intent::from_json(&json!({ "rule": "r", "a": 1, "b": 2 })),
        None
    );
    assert_eq!(Intent::from_json(&json!({ "withhold": "event" })), None);
}

#[test]
fn a_then_named_rule_is_refused() {
    let error = Rule::parse(
        "bad",
        r#"{ "on": "x", "strategy": [{ "id": "s", "judge": "system-one",
             "ask": { "outcomes": ["yes", "no"], "instructions": "?" } }],
             "then": [{ "if": { "s": "no" }, "rule": "event" }] }"#,
    )
    .unwrap_err();

    assert!(error.to_string().contains("cannot name"), "{error}");
}

#[test]
fn a_rule_over_a_limit_is_refused_naming_it() {
    let steps: Vec<Value> = (0..33)
        .map(|index| {
            json!({ "id": format!("s{index}"), "judge": "system-one",
                             "ask": { "outcomes": ["yes", "no"], "instructions": "?" } })
        })
        .collect();
    let text = json!({ "on": "x", "strategy": steps }).to_string();

    let error = Rule::parse("long", &text).unwrap_err();

    assert_eq!(
        Rejection::from(error).to_error()["data"]["limit"],
        json!("steps")
    );
}

#[test]
fn a_run_reports_its_rounds_and_settled_steps_to_three_decimals() {
    let navigate = Navigate::parse(&json!({
        "protocol": 1,
        "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
        "strategy": [
            { "id": "done", "judge": "quick",
              "ask": { "outcomes": ["low", "medium", "high"], "instructions": "How far?" } },
            { "id": "safe", "judge": "quick",
              "ask": { "outcomes": ["yes", "no"], "instructions": "Safe?" } }
        ]
    }))
    .unwrap();
    let Target::Strategy(strategy) = navigate.target() else {
        panic!("an inline strategy")
    };
    let bar = Confidence::Bar(Milli::new(700).unwrap());
    let by = AnsweredBy {
        model: "stand-in".into(),
        placeholder: true,
    };
    let judge = |question: &Question| {
        if question.outcomes().len() == 2 {
            return Outcome::Failed(Failure::Timeout);
        }
        let p = [("low", 0.12), ("medium", 0.45), ("high", 0.43)];
        let p = Probabilities::read(question.outcomes(), p).unwrap();
        Outcome::Value(Answer::settle(question, p, &bar, by.clone()))
    };

    let run = strategy
        .plan()
        .run(&Panel::new().with("quick", &judge))
        .unwrap();
    let events = progress::run_events(Some("r"), &run.events);

    assert_eq!(
        events,
        [
            json!({ "event": "round_ready", "rule": "r", "round": 1, "steps": ["done", "safe"] }),
            json!({ "event": "step_settled", "rule": "r", "step": "done",
                    "answer": { "outcome": "unsure", "p": { "low": 0.12, "medium": 0.45, "high": 0.43 },
                                "level": 1.31, "model": "stand-in", "placeholder": true } }),
            json!({ "event": "step_settled", "rule": "r", "step": "safe", "failed": "timeout" }),
        ]
    );
    assert_eq!(
        progress::verdict(&run.verdict)["safe"],
        json!({ "failed": "timeout" })
    );
    assert_eq!(
        progress::rule_matched("r"),
        json!({ "event": "rule_matched", "rule": "r" })
    );
}
