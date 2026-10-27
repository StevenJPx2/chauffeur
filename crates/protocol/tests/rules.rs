#![expect(
    clippy::unwrap_used,
    reason = "the helpers fail the test on a broken fixture"
)]

use std::path::PathBuf;

use chauffeur_core::{Failure, Outcome, Panel};
use chauffeur_judges::{Answer, AnsweredBy, Confidence, Milli, Probabilities, Question, Tier};
use chauffeur_protocol::{Event, Rule, RuleError, Then};

fn shipped(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../distribution/rules")
        .join(name);
    std::fs::read_to_string(path).unwrap()
}

fn commit_rule() -> Rule {
    Rule::parse(
        "conventional-commits",
        &shipped("conventional-commits.json"),
    )
    .unwrap()
}

fn tool_after(command: &str) -> Event {
    let event = serde_json::json!({
        "event": "tool.after", "session": "s1", "tool": "shell", "input": { "command": command }
    });
    Event::parse(&event.to_string()).unwrap()
}

/// What the rule returns when its `quick` judge gives `yes` with probability
/// `reply`, or fails.
fn then_for(rule: &Rule, reply: Result<f64, Failure>) -> Option<String> {
    let confidence = Confidence::Bar(Milli::new(700).unwrap());
    let by = AnsweredBy {
        model: "jev".into(),
        placeholder: true,
    };
    let judge = |question: &Question| match reply {
        Ok(yes) => {
            let p = Probabilities::read(question.outcomes(), [("yes", yes), ("no", 1.0 - yes)])
                .unwrap();
            Outcome::Value(Answer::settle(question, p, &confidence, by.clone()))
        }
        Err(failure) => Outcome::Failed(failure),
    };
    let run = rule
        .plan()
        .run(&Panel::new().with("quick", &judge))
        .unwrap();
    Then::first_met(rule.then(), &run.verdict)
        .map(|then| format!("{} {}", then.name(), then.payload()))
}

#[test]
fn the_shipped_commit_rule_hooks_only_commits() {
    let rule = commit_rule();
    let read = r#"{ "event": "tool.after", "tool": "read", "input": { "path": "git commit" } }"#;
    let before =
        r#"{ "event": "tool.before", "tool": "shell", "input": { "command": "git commit" } }"#;

    assert!(rule.hooks(&tool_after("git commit -m 'fixed stuff'")));
    assert!(!rule.hooks(&tool_after("ls -la")));
    assert!(!rule.hooks(&tool_after("cargo test")));
    assert!(
        !rule.hooks(&Event::parse(read).unwrap()),
        "only shell calls"
    );
    assert!(
        !rule.hooks(&Event::parse(before).unwrap()),
        "only after the call"
    );
    assert_eq!(
        rule.cooldown().map(|cooldown| cooldown.as_secs()),
        Some(120)
    );
    assert_eq!(
        rule.judges().get("quick").map(|judge| judge.tier),
        Some(Tier::SystemOne)
    );
}

#[test]
fn the_nudge_goes_out_only_on_a_confident_yes() {
    let rule = commit_rule();

    let nudge = then_for(&rule, Ok(0.94)).unwrap();
    assert!(nudge.starts_with("message.append "), "{nudge}");
    assert!(nudge.contains(r#""delivery":"now""#), "{nudge}");
    assert_eq!(then_for(&rule, Ok(0.06)), None, "a conventional message");
    assert_eq!(then_for(&rule, Ok(0.58)), None, "unsure sends nothing");
    assert_eq!(
        then_for(&rule, Err(Failure::Unavailable)),
        None,
        "a failure sends nothing"
    );
}

#[test]
fn a_tool_event_gives_its_call_and_output_as_evidence() {
    let event = Event::parse(
        r#"{ "event": "tool.after", "session": "s1", "tool": "shell",
             "input": { "command": "git commit -m 'fixed stuff'", "timeout": 30 },
             "output": "1 file changed" }"#,
    )
    .unwrap();

    let call = "tool: shell\ncommand: git commit -m 'fixed stuff'\ntimeout: 30\n";
    assert_eq!(
        event.evidence(),
        [
            ("tool_call".to_owned(), call.to_owned()),
            ("tool_output".to_owned(), "1 file changed".to_owned()),
            ("session".to_owned(), "s1".to_owned()),
        ]
    );
    assert_eq!(event.session(), Some("s1"));
    assert!(Event::parse(r#"{ "tool": "shell" }"#).is_err());
}

const ASK: &str = r#"{ "outcomes": ["yes", "no"], "instructions": "Is it?" }"#;

/// A rule with one step `s` asking `ask` of `judge`, and the given `judges`
/// and `then`.
fn rule_text(judges: &str, judge: &str, ask: &str, then: &str) -> String {
    format!(
        r#"{{ "on": "x", "judges": {{ {judges} }},
             "strategy": [{{ "id": "s", "judge": "{judge}", "ask": {ask} }}], "then": [{then}] }}"#
    )
}

fn assert_refused(text: &str, expected: &str) {
    let error = Rule::parse("bad", text).unwrap_err().to_string();
    assert!(error.starts_with("rule `bad`"), "{error}");
    assert!(error.contains(expected), "{error} lacks {expected}");
}

#[test]
fn a_step_with_an_unknown_judge_ask_or_bad_question_is_refused() {
    assert_refused(
        &rule_text("", "nobody", ASK, ""),
        "step `s` names judge `nobody`",
    );
    assert_refused(
        &rule_text("", "system-one", r#""missing""#, ""),
        "names ask `missing`",
    );
    let one_outcome = r#"{ "outcomes": ["yes"], "instructions": "?" }"#;
    assert_refused(
        &rule_text("", "system-one", one_outcome, ""),
        "2 to 64 outcomes",
    );
    assert_refused(r#"{ "on": "x", "strategy": [] }"#, "at least one step");
    assert_refused(
        &rule_text("", "system-one", ASK, "").replace("\"then\"", "\"typo\""),
        "unknown field `typo`",
    );
    assert!(matches!(
        Rule::parse("bad", "not json"),
        Err(RuleError::Parse { .. })
    ));
}

#[test]
fn a_confidence_that_does_not_fit_is_refused() {
    let over = r#""q": { "tier": "system-one", "confidence": 1.5 }"#;
    let partial = r#""q": { "tier": "system-one", "confidence": { "yes": 0.6 } }"#;

    assert_refused(
        &rule_text(over, "q", ASK, ""),
        "confidence 1.5 is not a probability",
    );
    assert_refused(
        &rule_text(partial, "q", ASK, ""),
        "no bar for the outcome `no`",
    );
}

#[test]
fn a_then_entry_naming_what_the_strategy_lacks_is_refused() {
    let maybe = r#"{ "if": { "s": "maybe" }, "withhold": "event" }"#;
    let other_step = r#"{ "if": { "t": "yes" }, "withhold": "event" }"#;
    let nothing = r#"{ "if": { "s": "yes" } }"#;

    assert_refused(
        &rule_text("", "system-one", ASK, maybe),
        "expects `maybe` from step `s`",
    );
    assert_refused(
        &rule_text("", "system-one", ASK, other_step),
        "names step `t`",
    );
    assert_refused(&rule_text("", "system-one", ASK, nothing), "exactly one");
}

#[test]
fn unsure_and_failed_can_be_expected_in_then() {
    let quick = r#""quick": { "tier": "system-one", "confidence": 0.7 }"#;
    let then = r#"{ "if": { "s": "unsure" }, "ask": "unsure" }, { "if": { "s": "failed" }, "ask": "failed" }"#;
    let rule = Rule::parse("fallbacks", &rule_text(quick, "quick", ASK, then)).unwrap();

    assert_eq!(then_for(&rule, Ok(0.5)).as_deref(), Some(r#"ask "unsure""#));
    assert_eq!(
        then_for(&rule, Err(Failure::Timeout)).as_deref(),
        Some(r#"ask "failed""#)
    );
    assert_eq!(then_for(&rule, Ok(0.9)), None);
}
