//! The functions the playground page calls, run natively with the page's
//! own examples.
#![expect(
    clippy::unwrap_used,
    reason = "a reply that is not JSON fails the test"
)]

use chauffeur_playground::{settle, version};
use serde_json::{Value, json};

fn settled(input: &Value) -> Value {
    serde_json::from_str(&settle(&input.to_string())).unwrap()
}

#[test]
fn a_likely_outcome_above_its_bar_settles_with_every_probability_and_the_level() {
    let reply = settled(
        &json!({ "outcomes": ["yes", "no"], "p": { "yes": 0.94, "no": 0.06 }, "confidence": 0.7 }),
    );

    assert_eq!(
        reply,
        json!({ "answer": { "outcome": "yes", "p": [{ "outcome": "yes", "p": 0.94 }, { "outcome": "no", "p": 0.06 }], "level": 0.06 } })
    );
}

#[test]
fn a_tie_is_unsure_and_a_per_outcome_bar_applies_to_its_outcome() {
    let tie = settled(&json!({ "outcomes": ["yes", "no"], "p": { "yes": 0.5, "no": 0.5 } }));
    assert_eq!(tie["answer"]["outcome"], "unsure");

    let below = settled(
        &json!({ "outcomes": ["yes", "no"], "p": { "yes": 0.71, "no": 0.29 },
                                 "confidence": { "yes": 0.8, "no": 0.95 } }),
    );
    assert_eq!(below["answer"]["outcome"], "unsure");
}

#[test]
fn a_reply_missing_an_outcome_failed_and_a_bad_question_is_an_error() {
    let missing =
        settled(&json!({ "outcomes": ["yes", "no"], "p": { "yes": 0.9 }, "confidence": 0.7 }));
    assert_eq!(missing, json!({ "failed": "invalid" }));

    let bar = settled(
        &json!({ "outcomes": ["yes", "no"], "p": { "yes": 0.9, "no": 0.1 }, "confidence": 1.4 }),
    );
    assert_eq!(
        bar["error"],
        "confidence 1.4 is not a probability from 0 to 1"
    );

    let unsure =
        settled(&json!({ "outcomes": ["yes", "unsure"], "p": { "yes": 0.9, "unsure": 0.1 } }));
    assert!(unsure["error"].is_string(), "{unsure}");
    assert!(settled(&json!({ "outcomes": "yes" }))["error"].is_string());
}

#[test]
fn the_version_is_the_workspace_s() {
    assert_eq!(version(), env!("CARGO_PKG_VERSION"));
}
