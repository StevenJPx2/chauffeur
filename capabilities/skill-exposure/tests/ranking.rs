//! With the daemon's ranking, a prompt judges its pinned skills plus the
//! closest few; without one, every skill up to the cap, pinned first. A
//! skill the prompt did not judge stays available for a mid-turn hand-over.

use chauffeur_capability_skill_exposure::{SkillExposure, SkillExposureConfig};
use chauffeur_core::{CatalogEntry, Engine, Judging, Signal, SignalKind, Step};

fn skill(id: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        description: format!("{id} workflow"),
        bytes: 1_000,
    }
}

fn prompt(text: &str, skills: &[&str], ranking: &[&str]) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::UserMessage {
            text: text.into(),
            first_in_context: true,
            skills: skills.iter().copied().map(skill).collect(),
            tools: Vec::new(),
            model: None,
            code_mode: Vec::new(),
            workspace: "/work/hpdp-overlay".into(),
            ranking: ranking.iter().map(|id| (*id).to_string()).collect(),
        },
    }
}

fn engine(judged: usize, always: &[&str]) -> Engine {
    let mut config = SkillExposureConfig::default();

    config.ranking.judged = judged;
    config.ranking.always = always.iter().map(|id| (*id).to_string()).collect();

    Engine::hosted(vec![Box::new(Judging::new(SkillExposure::new(config)))]).unwrap()
}

fn asked(step: Step) -> Vec<String> {
    let Step::Ask { questions, .. } = step else {
        panic!("expected questions, got {step:?}")
    };

    questions
        .into_iter()
        .map(|question| {
            question
                .id
                .trim_start_matches("skill-exposure/")
                .to_string()
        })
        .collect()
}

const SKILLS: [&str; 6] = [
    "vite",
    "code-discipline",
    "hpdp-overlay",
    "slack-cli",
    "nuxt",
    "vitest",
];

#[test]
fn a_ranked_prompt_judges_pinned_skills_and_only_the_closest_others() {
    let mut engine = engine(2, &["code-discipline"]);
    let ranking = [
        "vitest",
        "nuxt",
        "vite",
        "slack-cli",
        "code-discipline",
        "hpdp-overlay",
    ];
    let ids = asked(
        engine
            .begin(&prompt("send the slack update", &SKILLS, &ranking))
            .unwrap(),
    );

    // Always-on, named, and project skills first, closest first; then the two
    // closest others. vite ranked third, so it is not judged.
    assert_eq!(
        ids,
        [
            "named:slack-cli",
            "skill:code-discipline",
            "project:hpdp-overlay",
            "skill:vitest",
            "skill:nuxt",
        ]
    );
}

#[test]
fn without_a_ranking_every_skill_is_judged_with_pinned_ones_first() {
    let mut engine = engine(2, &["code-discipline"]);
    let ids = asked(
        engine
            .begin(&prompt("fix the build", &SKILLS, &[]))
            .unwrap(),
    );

    assert_eq!(
        ids,
        [
            "skill:code-discipline",
            "project:hpdp-overlay",
            "skill:vite",
            "skill:slack-cli",
            "skill:nuxt",
            "skill:vitest",
        ]
    );
}

#[test]
fn a_skill_the_prompt_did_not_judge_can_still_be_handed_over_mid_turn() {
    let mut engine = engine(1, &[]);
    let ranking = [
        "vitest",
        "vite",
        "nuxt",
        "slack-cli",
        "code-discipline",
        "hpdp-overlay",
    ];

    engine
        .begin(&prompt("fix the build", &SKILLS, &ranking))
        .unwrap();
    engine.finish(Ok(Vec::new()), 1);
    let tool = Signal {
        agent_id: "ses".into(),
        at: 120,
        kind: SignalKind::ToolResult {
            tool: "shell".into(),
            ok: true,
            workspace: String::new(),
            subagent: false,
            input: r#"{"command":"curl https://slack.com/api/chat.postMessage"}"#.into(),
            error: String::new(),
            user_request: String::new(),
            evidence: String::new(),
            candidates: Vec::new(),
        },
    };
    let Step::Ask { questions, .. } = engine.begin(&tool).unwrap() else {
        panic!("expected a drift question")
    };
    let chauffeur_core::QuestionKind::Choice { options } = &questions[0].kind else {
        panic!("expected a choice")
    };
    let values: Vec<&str> = options.iter().map(|option| option.value.as_str()).collect();

    // Every unattached skill is a hand-over option, closest first, including
    // vite and nuxt, which the prompt ranked past `judged` and never asked about.
    assert_eq!(values[..4], ["vitest", "vite", "nuxt", "slack-cli"]);
}

#[test]
fn past_the_cap_an_unranked_catalog_keeps_its_pinned_skills() {
    let mut skills: Vec<String> = (0..70).map(|n| format!("extra-{n}")).collect();

    skills.push("code-discipline".into());
    let names: Vec<&str> = skills.iter().map(String::as_str).collect();
    let mut engine = engine(16, &["code-discipline"]);
    let ids = asked(engine.begin(&prompt("fix the build", &names, &[])).unwrap());

    assert_eq!(ids.len(), 64);
    assert_eq!(ids[0], "skill:code-discipline");
}
