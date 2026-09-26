//! A prompt can need two skills, such as a Slack thread's `slack-cli` and the
//! project's own skill. A skill the prompt's facts point at gets its own
//! question; otherwise a first pick brings one more round.

use chauffeur_capability_skill_exposure::SkillExposure;
use chauffeur_core::{
    Answer, AnswerValue, CatalogEntry, Effect, Engine, QuestionKind, Signal, SignalKind, Step,
};

const PROJECT: &str = "/Users/me/Projects/hpdp-overlay/ADEPT-45130";
const ELSEWHERE: &str = "/work/app";
const SLACK_LINK: &str =
    "https://adeptmind.slack.com/archives/C08ABCDEF12/p1790340000123456 Can you fix this?";
const PLAIN: &str = "Can you fix the failing checkout test?";

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

fn prompt(workspace: &str, text: &str, skills: Vec<CatalogEntry>) -> Signal {
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
            workspace: workspace.into(),
        },
    }
}

fn choose(id: &str, value: &str) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Choice(value.into()),
        confidence: Some(0.9),
    }
}

fn yes(id: &str, p: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

fn options(kind: &QuestionKind) -> Vec<&str> {
    let QuestionKind::Choice { options } = kind else {
        panic!("expected a choice")
    };

    options.iter().map(|option| option.value.as_str()).collect()
}

fn attached(step: &Step) -> Vec<&str> {
    let Step::Done(effects) = step else {
        panic!("expected effects, got {step:?}")
    };

    effects
        .iter()
        .flat_map(|effect| match effect {
            Effect::Context { skills, .. } => skills.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn engine() -> Engine {
    Engine::hosted(vec![Box::new(SkillExposure::default())]).unwrap()
}

/// Round one of a plain prompt elsewhere, answered with `pick`.
fn first_round(engine: &mut Engine, pick: &str) -> Step {
    engine.begin(&prompt(ELSEWHERE, PLAIN, catalog())).unwrap();
    engine.finish(Ok(vec![choose("skill-exposure/pick", pick)]), 1)
}

/// Round one of the Slack link in the project, with these P(yes) answers.
fn slack_in_project(named: f32, project: f32) -> Step {
    slack_round(&mut engine(), named, project)
}

fn slack_round(engine: &mut Engine, named: f32, project: f32) -> Step {
    engine
        .begin(&prompt(PROJECT, SLACK_LINK, catalog()))
        .unwrap();
    engine.finish(
        Ok(vec![
            choose("skill-exposure/pick", "none"),
            yes("skill-exposure/named:slack-cli", named),
            yes("skill-exposure/project:hpdp-overlay", project),
        ]),
        1,
    )
}

#[test]
fn a_slack_link_in_a_project_asks_about_both_skills_directly() {
    let mut engine = engine();
    let Step::Ask { state, questions } = engine
        .begin(&prompt(PROJECT, SLACK_LINK, catalog()))
        .unwrap()
    else {
        panic!("expected questions")
    };
    let ids: Vec<&str> = questions
        .iter()
        .map(|question| question.id.as_str())
        .collect();

    // The link reaches Jev whole; the skill it names and the project's own
    // skill each get a question, the rest are in the pick.
    assert!(state.contains("slack.com/archives/C08ABCDEF12/p1790340000123456"));
    assert_eq!(
        ids,
        [
            "skill-exposure/pick",
            "skill-exposure/named:slack-cli",
            "skill-exposure/project:hpdp-overlay"
        ]
    );
    assert_eq!(options(&questions[0].kind), ["jira-cli", "none"]);
    assert!(questions[2].instructions.contains(PROJECT));
    assert_eq!(
        attached(&slack_in_project(0.83, 0.55)),
        ["slack-cli", "hpdp-overlay"]
    );
}

#[test]
fn a_project_skill_is_withheld_only_on_a_confident_no_and_a_named_one_needs_a_yes() {
    // Unsure about the project still attaches it; an unsure named skill does
    // not, so one skill brings the second pick.
    let mut unsure = engine();
    let Step::Ask { questions, .. } = slack_round(&mut unsure, 0.5, 0.45) else {
        panic!("expected the second pick")
    };

    assert_eq!(
        options(&questions[0].kind),
        ["slack-cli", "jira-cli", "none"]
    );
    assert_eq!(
        attached(&unsure.finish(Ok(vec![choose("skill-exposure/also", "none")]), 1)),
        ["hpdp-overlay"]
    );

    // A confident no withholds the project skill; nothing attached ends it.
    assert!(attached(&slack_in_project(0.5, 0.04)).is_empty());

    let mut failed = engine();

    failed
        .begin(&prompt(PROJECT, SLACK_LINK, catalog()))
        .unwrap();
    assert!(attached(&failed.finish(Err("down".into()), 1)).is_empty());
}

#[test]
fn the_second_pick_stays_within_the_prompt_budget() {
    let mut engine = engine();

    engine
        .begin(&prompt(
            ELSEWHERE,
            PLAIN,
            vec![
                skill("slack-cli", 1_761),
                skill("hpdp-overlay", 18_483),
                skill("rust-skills", 64_000),
            ],
        ))
        .unwrap();

    let Step::Ask { questions, .. } =
        engine.finish(Ok(vec![choose("skill-exposure/pick", "hpdp-overlay")]), 1)
    else {
        panic!("expected the second pick")
    };

    // 18.5 KB spent: a 64 KB skill no longer fits, so it is not offered.
    assert_eq!(options(&questions[0].kind), ["slack-cli", "none"]);
    assert_eq!(
        attached(&engine.finish(Ok(vec![choose("skill-exposure/also", "slack-cli")]), 1)),
        ["hpdp-overlay", "slack-cli"]
    );
}

#[test]
fn a_second_none_or_failure_keeps_the_first_and_a_first_none_asks_nothing_more() {
    let mut declined = engine();

    first_round(&mut declined, "slack-cli");
    assert_eq!(
        attached(&declined.finish(Ok(vec![choose("skill-exposure/also", "none")]), 1)),
        ["slack-cli"]
    );

    let mut failed = engine();

    first_round(&mut failed, "slack-cli");
    assert_eq!(
        attached(&failed.finish(Err("down".into()), 1)),
        ["slack-cli"]
    );

    assert!(attached(&first_round(&mut engine(), "none")).is_empty());
}
