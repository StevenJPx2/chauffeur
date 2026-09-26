//! A prompt, or the agent's own request, judges every offered skill in one
//! call; every skill it needs is attached, most likely first, within budget.

use chauffeur_capability_skill_exposure::{MAX_SIGNAL_BYTES, SkillExposure};
use chauffeur_core::{
    Answer, AnswerValue, CatalogEntry, Delivery, Effect, Engine, Judging, Signal, SignalKind, Step,
};

const PROJECT: &str = "/Users/me/Projects/hpdp-overlay/ADEPT-45130";
const SLACK_LINK: &str =
    "https://adeptmind.slack.com/archives/C08ABCDEF12/p1790340000123456 Can you fix this?";

fn skill(id: &str, bytes: u32) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        description: format!("{id} workflow"),
        bytes,
    }
}

fn catalog() -> Vec<CatalogEntry> {
    vec![
        skill("slack-cli", 1_761),
        skill("hpdp-overlay", 18_483),
        skill("jira-cli", 900),
    ]
}

fn prompt(text: &str, skills: Vec<CatalogEntry>) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::UserMessage {
            text: text.into(),
            first_in_context: true,
            skills,
            tools: Vec::new(),
            model: None,
            code_mode: Vec::new(),
            workspace: PROJECT.into(),
        },
    }
}

fn request(need: &str) -> Signal {
    Signal {
        kind: SignalKind::AgentRequest {
            need: need.into(),
            user_request: "file the bug".into(),
            tools: Vec::new(),
            code_mode: Vec::new(),
        },
        ..prompt("", Vec::new())
    }
}

fn yes(id: &str, p: f32) -> Answer {
    Answer {
        id: format!("skill-exposure/{id}"),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

fn engine() -> Engine {
    Engine::hosted(vec![Box::new(Judging::new(SkillExposure::default()))]).unwrap()
}

fn asked(step: &Step) -> Vec<&str> {
    let Step::Ask { questions, .. } = step else {
        panic!("expected questions, got {step:?}")
    };

    questions
        .iter()
        .map(|question| question.id.as_str())
        .collect()
}

fn attached(step: &Step) -> Vec<(&str, Delivery)> {
    let Step::Done(effects) = step else {
        panic!("expected effects, got {step:?}")
    };

    effects
        .iter()
        .flat_map(|effect| match effect {
            Effect::Context {
                skills, delivery, ..
            } => skills
                .iter()
                .map(|skill| (skill.as_str(), *delivery))
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}

/// The Slack link in the project, answered with P for each skill.
fn slack_in_project(slack: f32, project: f32, jira: f32) -> Step {
    let mut engine = engine();

    engine.begin(&prompt(SLACK_LINK, catalog())).unwrap();
    engine.finish(
        Ok(vec![
            yes("named:slack-cli", slack),
            yes("project:hpdp-overlay", project),
            yes("skill:jira-cli", jira),
        ]),
        1,
    )
}

#[test]
fn every_skill_is_asked_about_in_one_round_worded_by_its_facts() {
    let mut engine = engine();
    let Step::Ask { state, questions } = engine.begin(&prompt(SLACK_LINK, catalog())).unwrap()
    else {
        panic!("expected questions")
    };
    let ids: Vec<&str> = questions
        .iter()
        .map(|question| question.id.as_str())
        .collect();

    // The link reaches Jev whole; the Slack skill it names and the project's
    // own skill are worded by those facts.
    assert!(state.contains("slack.com/archives/C08ABCDEF12/p1790340000123456"));
    assert_eq!(
        ids,
        [
            "skill-exposure/named:slack-cli",
            "skill-exposure/project:hpdp-overlay",
            "skill-exposure/skill:jira-cli"
        ]
    );
    assert!(questions[1].instructions.contains(PROJECT));
}

#[test]
fn every_needed_skill_attaches_to_the_prompt_most_likely_first() {
    assert_eq!(
        attached(&slack_in_project(0.83, 0.55, 0.1)),
        [
            ("slack-cli", Delivery::Prompt),
            ("hpdp-overlay", Delivery::Prompt)
        ]
    );
}

#[test]
fn the_project_skill_needs_only_the_absence_of_a_confident_no() {
    assert_eq!(
        attached(&slack_in_project(0.5, 0.45, 0.1)),
        [("hpdp-overlay", Delivery::Prompt)]
    );
    assert!(attached(&slack_in_project(0.5, 0.04, 0.1)).is_empty());

    let mut failed = engine();

    failed.begin(&prompt(SLACK_LINK, catalog())).unwrap();
    assert!(attached(&failed.finish(Err("down".into()), 1)).is_empty());
}

#[test]
fn skills_attach_within_the_prompt_budget() {
    let mut engine = engine();
    let large = u32::try_from(MAX_SIGNAL_BYTES - 1_000).unwrap();

    engine
        .begin(&prompt(
            "Fix the checkout test",
            vec![
                skill("rust-skills", large),
                skill("jira-cli", 900),
                skill("wrangler", 5_000),
            ],
        ))
        .unwrap();

    // The likeliest fits; the next is over budget and skipped for a smaller one.
    let step = engine.finish(
        Ok(vec![
            yes("skill:rust-skills", 0.95),
            yes("skill:wrangler", 0.9),
            yes("skill:jira-cli", 0.8),
        ]),
        1,
    );

    assert_eq!(
        attached(&step),
        [
            ("rust-skills", Delivery::Prompt),
            ("jira-cli", Delivery::Prompt)
        ]
    );
}

#[test]
fn the_agents_request_is_answered_in_the_running_turn_and_a_skill_is_given_once() {
    let mut engine = engine();

    engine
        .begin(&prompt("Fix the checkout test", catalog()))
        .unwrap();
    engine.finish(
        Ok(vec![
            yes("skill:slack-cli", 0.1),
            yes("skill:hpdp-overlay", 0.1),
            yes("skill:jira-cli", 0.1),
        ]),
        1,
    );

    let ask = engine
        .begin(&request("create a Jira issue from the command line"))
        .unwrap();

    assert_eq!(
        asked(&ask),
        [
            "skill-exposure/skill:slack-cli",
            "skill-exposure/skill:hpdp-overlay",
            "skill-exposure/skill:jira-cli"
        ]
    );
    assert_eq!(
        attached(&engine.finish(
            Ok(vec![
                yes("skill:slack-cli", 0.1),
                yes("skill:hpdp-overlay", 0.1),
                yes("skill:jira-cli", 0.9)
            ]),
            1
        )),
        [("jira-cli", Delivery::Steer)]
    );
    assert_eq!(
        asked(&engine.begin(&request("the jira skill again")).unwrap()),
        [
            "skill-exposure/skill:slack-cli",
            "skill-exposure/skill:hpdp-overlay"
        ]
    );
}
