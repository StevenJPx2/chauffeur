//! The agent asks for a skill: System One picks one it has not been given, or
//! none, and the skill answers the request in the running turn.

use chauffeur_capability_skill_exposure::SkillExposure;
use chauffeur_core::{
    Answer, AnswerValue, Capability, CatalogEntry, Delivery, Effect, Plan, QuestionKind, Signal,
    SignalKind, Situation,
};

fn skill(id: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        description: format!("{id} workflow"),
        bytes: 100,
    }
}

fn signal(kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind,
    }
}

fn message() -> Signal {
    signal(SignalKind::UserMessage {
        text: "file the bug in jira".into(),
        first_in_context: false,
        skills: vec![skill("jira-cli"), skill("slack-cli")],
        tools: Vec::new(),
        model: None,
        code_mode: Vec::new(),
        workspace: String::new(),
    })
}

fn request() -> Signal {
    signal(SignalKind::AgentRequest {
        need: "how to create a Jira issue from the command line".into(),
        user_request: "file the bug in jira".into(),
        tools: Vec::new(),
        code_mode: Vec::new(),
    })
}

fn choice(id: &str, value: &str) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Choice(value.into()),
        confidence: Some(0.9),
    }
}

#[test]
fn the_agent_asks_and_gets_one_skill_it_was_not_given() {
    let mut exposure = SkillExposure::default();

    exposure.plan(&Situation::default(), &message());
    exposure.decide(&message(), Some(&[choice("pick", "slack-cli")]));

    let Plan::Ask(questions) = exposure.plan(&Situation::default(), &request()) else {
        panic!("expected the request question")
    };
    let QuestionKind::Choice { options } = &questions[0].kind else {
        panic!("expected a choice")
    };
    let offered: Vec<&str> = options.iter().map(|option| option.value.as_str()).collect();

    assert_eq!(questions[0].id, "ask");
    assert!(questions[0].instructions.contains("create a Jira issue"));
    assert_eq!(offered, ["jira-cli", "none"]);
    assert_eq!(
        exposure.decide(&request(), Some(&[choice("ask", "jira-cli")])),
        vec![Effect::Context {
            agent_id: "ses".into(),
            delivery: Delivery::Steer,
            label: "skill jira-cli".into(),
            skills: vec!["jira-cli".into()],
            text: None,
        }]
    );
    // Given once, it is not offered again.
    assert!(matches!(
        exposure.plan(&Situation::default(), &request()),
        Plan::Skip
    ));
}

#[test]
fn none_or_a_failure_hands_over_nothing() {
    let mut exposure = SkillExposure::default();

    exposure.plan(&Situation::default(), &message());

    assert!(
        exposure
            .decide(&request(), Some(&[choice("ask", "none")]))
            .is_empty()
    );
    assert!(exposure.decide(&request(), None).is_empty());
}
