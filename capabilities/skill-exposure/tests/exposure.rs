use chauffeur_capability_skill_exposure::{
    DRIFT_COOLDOWN_SECS, MAX_SIGNAL_BYTES, NONE, SkillExposure,
};
use chauffeur_core::{
    Answer, AnswerValue, Capability, CatalogEntry, Delivery, Effect, Plan, QuestionKind, Signal,
    SignalKind, Situation,
};

fn skill(id: &str, bytes: u64) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        description: format!("{id} workflow"),
        bytes: u32::try_from(bytes).unwrap(),
    }
}

fn signal(at: u64, kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at,
        kind,
    }
}

fn message(skills: Vec<CatalogEntry>) -> Signal {
    signal(
        1,
        SignalKind::UserMessage {
            text: "post the release notes to twitter".into(),
            first_in_context: false,
            skills,
            tools: Vec::new(),
            model: None,
            code_mode: Vec::new(),
            workspace: String::new(),
        },
    )
}

fn tool_result(at: u64) -> Signal {
    signal(
        at,
        SignalKind::ToolResult {
            tool: "shell".into(),
            ok: true,
            workspace: String::new(),
            input: r#"{"command":"browser-harness open https://x.com"}"#.into(),
            error: String::new(),
            user_request: String::new(),
            evidence: String::new(),
            candidates: Vec::new(),
        },
    )
}

/// A successful `gh` call, which does not drive a browser.
fn gh_call(at: u64) -> Signal {
    let mut tool = tool_result(at);

    if let SignalKind::ToolResult { input, .. } = &mut tool.kind {
        *input = r#"{"command":"gh pr view 42"}"#.into();
    }

    tool
}

fn turn_end(at: u64) -> Signal {
    signal(
        at,
        SignalKind::TurnEnd {
            workspace: String::new(),
            user_request: String::new(),
        },
    )
}

fn choice(id: &str, value: &str, confidence: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Choice(value.into()),
        confidence: Some(confidence),
    }
}

/// A skill attached to the user's message.
fn attached(skill: &str) -> Vec<Effect> {
    vec![Effect::Context {
        agent_id: "ses".into(),
        delivery: Delivery::Prompt,
        label: format!("skill {skill}"),
        skills: vec![skill.into()],
        text: None,
    }]
}

/// A drift hand-over into the running turn.
fn handed_over(skill: &str) -> Vec<Effect> {
    vec![Effect::Context {
        agent_id: "ses".into(),
        delivery: Delivery::Steer,
        label: format!("skill {skill}"),
        skills: vec![skill.into()],
        text: Some(format!(
            "Chauffeur: the {skill} skill fits this work better than the current approach."
        )),
    }]
}

/// The (question id, option values) of a single-question plan.
fn offered(plan: &Plan) -> (String, Vec<String>) {
    let Plan::Ask(questions) = plan else {
        panic!("expected a question, got {plan:?}")
    };
    let [question] = questions.as_slice() else {
        panic!("expected one question")
    };
    let QuestionKind::Choice { options } = &question.kind else {
        panic!("expected a choice")
    };

    (
        question.id.clone(),
        options.iter().map(|option| option.value.clone()).collect(),
    )
}

#[test]
fn a_user_message_asks_one_choice_over_unique_skills_plus_none() {
    let mut exposure = SkillExposure::default();
    let plan = exposure.plan(
        &Situation::default(),
        &message(vec![
            skill("slack", 10),
            skill("jira", 10),
            skill("slack", 10),
        ]),
    );

    // Neither is named in the request, so both are in the one choice.
    assert_eq!(
        offered(&plan),
        (
            "pick".into(),
            vec!["slack".into(), "jira".into(), NONE.into()]
        )
    );
}

#[test]
fn attaches_the_confident_choice_only() {
    let mut exposure = SkillExposure::default();
    let signal = message(vec![
        skill("twitter", 10),
        skill("huge", MAX_SIGNAL_BYTES + 1),
    ]);

    exposure.plan(&Situation::default(), &signal);

    assert_eq!(
        exposure.decide(&signal, Some(&[choice("pick", "twitter", 0.9)])),
        attached("twitter")
    );
    assert!(
        exposure
            .decide(&signal, Some(&[choice("pick", NONE, 0.9)]))
            .is_empty()
    );
    assert!(
        exposure
            .decide(&signal, Some(&[choice("pick", "huge", 0.9)]))
            .is_empty()
    );
    assert!(exposure.decide(&signal, None).is_empty());
}

#[test]
fn a_tool_result_checks_for_drift_with_a_cooldown_and_never_reoffers() {
    let mut exposure = SkillExposure::default();

    exposure.plan(
        &Situation::default(),
        &message(vec![skill("twitter", 10), skill("slack", 10)]),
    );

    let first = tool_result(10);
    assert_eq!(
        offered(&exposure.plan(&Situation::default(), &first)).0,
        "drift"
    );
    assert_eq!(
        exposure.decide(&first, Some(&[choice("drift", "twitter", 0.8)])),
        handed_over("twitter")
    );

    // Within the cooldown a tool result is not judged; a turn end always is.
    assert_eq!(
        exposure.plan(&Situation::default(), &tool_result(20)),
        Plan::Skip
    );

    let (_, options) = offered(&exposure.plan(&Situation::default(), &turn_end(21)));
    assert_eq!(options, vec!["slack".to_string(), NONE.into()]);

    // A turn end without a new tool result since the last check is not judged.
    assert_eq!(
        exposure.plan(&Situation::default(), &turn_end(22)),
        Plan::Skip
    );

    let later = tool_result(21 + DRIFT_COOLDOWN_SECS);
    assert_eq!(
        offered(&exposure.plan(&Situation::default(), &later)).0,
        "drift"
    );
}

#[test]
fn a_weak_browser_hand_over_after_effective_gh_use_stays_silent() {
    let mut exposure = SkillExposure::default();
    let tool = gh_call(10);

    exposure.plan(
        &Situation::default(),
        &message(vec![skill("browser-harness", 10)]),
    );

    assert_eq!(exposure.plan(&Situation::default(), &tool), Plan::Skip);
    assert!(
        exposure
            .decide(&tool, Some(&[choice("drift", "browser-harness", 0.43)]))
            .is_empty()
    );
    assert_eq!(
        exposure.plan(&Situation::default(), &turn_end(11)),
        Plan::Skip
    );
}

#[test]
fn browser_handoff_requires_actual_browser_use_during_drift() {
    let mut exposure = SkillExposure::default();

    exposure.plan(
        &Situation::default(),
        &message(vec![skill("browser-harness", 10), skill("twitter", 10)]),
    );

    let (id, options) = offered(&exposure.plan(&Situation::default(), &gh_call(10)));
    assert_eq!(id, "drift");
    assert_eq!(options, vec!["twitter", NONE]);

    let Plan::Ask(questions) = exposure.plan(&Situation::default(), &tool_result(80)) else {
        panic!("expected browser drift")
    };
    assert!(questions[0].instructions.contains("browser-harness open"));
    let (_, options) = offered(&Plan::Ask(questions));
    assert_eq!(options, vec!["browser-harness", "twitter", NONE]);
}

#[test]
fn skips_without_a_known_catalog() {
    let mut exposure = SkillExposure::default();

    assert_eq!(
        exposure.plan(&Situation::default(), &tool_result(1)),
        Plan::Skip
    );
    assert_eq!(
        exposure.plan(&Situation::default(), &message(Vec::new())),
        Plan::Skip
    );
}
