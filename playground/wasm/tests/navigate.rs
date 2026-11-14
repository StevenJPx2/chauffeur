//! The navigate tab's function, run natively with requests like the page's.
#![expect(
    clippy::unwrap_used,
    reason = "a reply that is not JSON fails the test"
)]

use chauffeur_playground::navigate;
use serde_json::{Value, json};

fn navigated(input: &Value) -> Value {
    serde_json::from_str(&navigate(&input.to_string())).unwrap()
}

fn triage() -> Value {
    json!({
        "on": "sourcefed.event",
        "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
        "strategy": [{ "id": "screen", "judge": "quick",
                       "ask": { "outcomes": ["yes", "no"], "evidence": ["event"], "instructions": "Show it?" } }],
        "then": [{ "if": { "screen": "no" }, "withhold": "event" }]
    })
}

/// An event for the triage rule, with `screen` as its step's reply, or none.
fn event(text: &str, screen: &Value) -> Value {
    let answers = if screen.is_null() {
        json!({})
    } else {
        json!({ "screen": screen })
    };
    json!({
        "request": { "protocol": 1, "on": "sourcefed.event", "evidence": { "event": text } },
        "rules": { "sourcefed/triage": triage() },
        "answers": answers
    })
}

#[test]
fn confident_noise_is_withheld_after_the_runs_events() {
    let reply = navigated(&event("thanks", &json!({ "yes": 0.06, "no": 0.94 })));

    let kinds: Vec<&Value> = reply["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| &event["event"])
        .collect();
    assert_eq!(
        kinds,
        [
            &json!("rule_matched"),
            &json!("round_ready"),
            &json!("step_settled")
        ]
    );
    assert_eq!(
        reply["result"],
        json!({ "intents": [{ "rule": "sourcefed/triage", "withhold": "event" }] })
    );
}

#[test]
fn unsure_a_failure_and_no_reply_give_no_intent() {
    for screen in [
        json!({ "yes": 0.48, "no": 0.52 }),
        json!({ "failed": "timeout" }),
        Value::Null,
    ] {
        let reply = navigated(&event("which doc?", &screen));

        assert_eq!(reply["result"], json!({ "intents": [] }), "{screen}");
    }
}

#[test]
fn an_inline_strategy_gives_its_verdict_and_a_rule_that_does_not_load_is_refused() {
    let reply = navigated(&json!({
        "request": { "protocol": 1, "strategy": [{ "id": "done", "judge": "system-one",
                     "ask": { "outcomes": ["yes", "no"], "instructions": "Finished?" } }] },
        "answers": { "done": { "yes": 0.9, "no": 0.1 } }
    }));
    assert_eq!(
        reply["result"]["verdict"]["done"]["answer"]["outcome"],
        "yes"
    );

    let refused = navigated(&json!({
        "request": { "protocol": 1, "on": "x" },
        "rules": { "broken": { "on": "x", "strategy": [] } }
    }));
    assert!(
        refused["refused"][0]
            .as_str()
            .unwrap()
            .contains("rule `broken`"),
        "{refused}"
    );
}

#[test]
fn a_request_over_a_limit_is_rejected_as_a_caller_sees_it() {
    let reply =
        navigated(&json!({ "request": { "protocol": 1, "on": "x", "deadline_ms": 30_001 } }));

    assert_eq!(reply["rejected"]["data"]["limit"], "deadline_ms");
    assert!(navigated(&json!({ "nope": 1 }))["error"].is_string());
}
