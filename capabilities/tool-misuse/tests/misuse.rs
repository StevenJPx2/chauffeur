use chauffeur_capability_tool_misuse::{MisuseContract, ToolMisuse};
use chauffeur_core::{
    Answer, AnswerValue, Capability, Delivery, Effect, Plan, Signal, SignalKind, Situation,
};
use serde_json::{Value, json};

fn contract(id: &str, handoff: Option<&str>, cooldown: u64) -> Value {
    json!({
        "schema_version": 1,
        "identity": { "id": id, "name": id, "version": "1.0.0" },
        "match": { "tools": ["shell"] },
        "question": format!("Does this call do {id}?"),
        "nudge_at_or_above": 0.7,
        "minimum_confidence": 0.4,
        "steer": format!("Stop doing {id}."),
        "handoff_skill": handoff,
        "cooldown_seconds": cooldown,
    })
}

fn parsed(value: &Value) -> Result<MisuseContract, String> {
    MisuseContract::from_json(&serde_json::to_vec(value).unwrap())
}

fn misuse() -> ToolMisuse {
    ToolMisuse::new(vec![
        parsed(&contract("hand-rolled-patch", None, 120)).unwrap(),
        parsed(&contract("slack-via-browser", Some("slack-cli"), 120)).unwrap(),
    ])
}

fn call(at: u64, tool: &str) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at,
        kind: SignalKind::ToolResult {
            tool: tool.into(),
            ok: true,
            input: r#"{"command":"python3 patch.py"}"#.into(),
            error: String::new(),
            user_request: String::new(),
            evidence: String::new(),
            candidates: Vec::new(),
        },
    }
}

fn yes(id: &str, p: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

#[test]
fn only_watched_tools_are_judged_one_question_per_contract() {
    let mut misuse = misuse();

    assert_eq!(
        misuse.plan(&Situation::default(), &call(1, "edit")),
        Plan::Skip
    );

    let Plan::Ask(questions) = misuse.plan(&Situation::default(), &call(1, "shell")) else {
        panic!("expected questions")
    };
    let ids: Vec<&str> = questions
        .iter()
        .map(|question| question.id.as_str())
        .collect();

    assert_eq!(ids, vec!["hand-rolled-patch", "slack-via-browser"]);
    assert!(
        questions[0]
            .instructions
            .contains("Does this call do hand-rolled-patch?")
    );
    assert!(questions[0].instructions.contains("python3 patch.py"));
}

#[test]
fn a_confirmed_misuse_steers_hands_over_its_skill_and_cools_down() {
    let mut misuse = misuse();

    assert_eq!(
        misuse.decide(
            &call(1, "shell"),
            Some(&[yes("hand-rolled-patch", 0.2), yes("slack-via-browser", 0.9)])
        ),
        vec![Effect::Context {
            agent_id: "ses".into(),
            delivery: Delivery::Steer,
            label: "shell check".into(),
            skills: vec!["slack-cli".into()],
            text: Some("Chauffeur: Stop doing slack-via-browser.".into())
        }]
    );

    // Cooling down, only the other contract is still asked.
    let Plan::Ask(questions) = misuse.plan(&Situation::default(), &call(60, "shell")) else {
        panic!("expected a question")
    };
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0].id, "hand-rolled-patch");
    assert!(misuse.decide(&call(60, "shell"), None).is_empty());
}

#[test]
fn contracts_are_strict() {
    let mut unknown = contract("ok-id", None, 0);
    unknown["extra"] = json!(true);
    let mut bad_bar = contract("ok-id", None, 0);
    bad_bar["nudge_at_or_above"] = json!(0.2);

    assert!(parsed(&contract("ok-id", None, 0)).is_ok());
    assert!(parsed(&unknown).is_err());
    assert!(parsed(&bad_bar).is_err());
    assert!(parsed(&contract("Bad ID", None, 0)).is_err());
}

#[test]
fn the_shipped_contracts_load() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills/misuse");
    let contracts = chauffeur_capability_tool_misuse::load_contracts(&dir)
        .expect("shipped contracts are valid");

    assert!(
        contracts
            .iter()
            .any(|contract| contract.identity.id == "hand-rolled-patch")
    );
    assert!(
        contracts
            .iter()
            .filter_map(|contract| contract.handoff_skill.as_deref())
            .all(|skill| ["twitter-cli", "slack-cli", "jira-cli"].contains(&skill))
    );
}
