use std::path::{Path, PathBuf};

use chauffeur_capability_project_skills::{ProjectSkills, load};
use chauffeur_core::{Answer, AnswerValue, Delivery, Effect, Engine, Signal, SignalKind, Step};

const CONTRACT: &str = r#"{
  "schema_version":1,"id":"hpdp-pr","match":{"event":"turn_end","tools_called_any":["edit"]},
  "steps":[
    {"id":"intent","question":"Does the user want a PR?","yes_at_or_above":0.7,"minimum_confidence":0.4},
    {"id":"ready","question":"Are tests complete?","yes_at_or_above":0.7,"minimum_confidence":0.4}
  ],
  "effect":{"label":"HPDP PR","delivery":"resume","text":"Open the PR after checks."},"once":true
}"#;

const ONE_STEP: &str = r#"{
  "schema_version":1,"id":"changelog","match":{"event":"turn_end"},
  "steps":[
    {"id":"notable","question":"Is the change notable?","yes_at_or_above":0.7,"minimum_confidence":0.4}
  ],
  "effect":{"label":"Changelog","delivery":"wait","text":"Add a changelog entry."}
}"#;

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "chauffeur-project-skills-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));

        std::fs::create_dir_all(path.join("project/.git")).unwrap();
        std::fs::create_dir_all(path.join("project/.chauffeur/skills")).unwrap();
        std::fs::create_dir_all(path.join("sibling/.git")).unwrap();
        std::fs::write(path.join("project/.chauffeur/skills/pr.json"), CONTRACT).unwrap();

        Self(path)
    }

    fn project(&self) -> PathBuf {
        self.0.join("project")
    }

    fn sibling(&self) -> PathBuf {
        self.0.join("sibling")
    }

    fn skill(&self, name: &str) -> PathBuf {
        self.project().join(".chauffeur/skills").join(name)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn engine() -> Engine {
    Engine::hosted(vec![Box::new(ProjectSkills::default())]).unwrap()
}

fn signal(kind: SignalKind) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind,
    }
}

fn edit(path: &Path) -> Signal {
    signal(SignalKind::ToolResult {
        tool: "edit".into(),
        ok: true,
        workspace: path.to_string_lossy().into_owned(),
        input: String::new(),
        error: String::new(),
        user_request: String::new(),
        evidence: String::new(),
        candidates: vec![],
    })
}

fn turn_end(path: &Path, request: &str) -> Signal {
    signal(SignalKind::TurnEnd {
        workspace: path.to_string_lossy().into_owned(),
        user_request: request.into(),
    })
}

fn answer(id: &str, probability: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Noul(probability),
        confidence: None,
    }
}

/// Confirm the intent step and return the readiness question's ID.
fn confirm_intent(engine: &mut Engine, turn: &Signal) -> String {
    let Step::Ask { questions, .. } = engine.begin(turn).unwrap() else {
        panic!("expected intent")
    };
    let Step::Ask { questions, .. } = engine.finish(Ok(vec![answer(&questions[0].id, 0.95)]), 1)
    else {
        panic!("expected readiness")
    };

    questions[0].id.clone()
}

#[test]
fn a_sibling_repository_contributes_no_project_skills() {
    let workspace = Workspace::new();
    let mut engine = engine();

    assert_eq!(
        engine.begin(&edit(&workspace.project())).unwrap(),
        Step::Done(vec![])
    );
    assert!(
        load(workspace.sibling().to_str().unwrap())
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        engine
            .begin(&turn_end(&workspace.sibling(), "open a PR"))
            .unwrap(),
        Step::Done(vec![])
    );
}

#[test]
fn an_explicit_user_opt_out_ends_the_contract_at_its_first_step() {
    let workspace = Workspace::new();
    let mut engine = engine();
    engine.begin(&edit(&workspace.project())).unwrap();

    let no_pr = turn_end(&workspace.project(), "do not create a PR");
    let Step::Ask { questions, .. } = engine.begin(&no_pr).unwrap() else {
        panic!("expected intent")
    };

    assert!(questions[0].instructions.contains("do not create a PR"));
    assert_eq!(
        engine.finish(Ok(vec![answer(&questions[0].id, 0.05)]), 1),
        Step::Done(vec![])
    );
}

#[test]
fn a_confirmed_contract_delivers_once_until_the_next_user_message() {
    let workspace = Workspace::new();
    let mut engine = engine();
    let edit = edit(&workspace.project());
    let pr = turn_end(&workspace.project(), "please open a PR once verified");
    engine.begin(&edit).unwrap();

    let ready = confirm_intent(&mut engine, &pr);
    assert_eq!(
        engine.finish(Ok(vec![answer(&ready, 0.2)]), 1),
        Step::Done(vec![])
    );

    let ready = confirm_intent(&mut engine, &pr);
    assert_eq!(ready, "project-skills/hpdp-pr/ready");
    let Step::Done(effects) = engine.finish(Ok(vec![answer(&ready, 0.95)]), 1) else {
        panic!("expected effect")
    };
    assert!(
        matches!(effects.as_slice(), [Effect::Context { delivery: Delivery::Resume, text: Some(text), skills, .. }] if text.contains("Open the PR") && skills.is_empty())
    );

    let mut restarted = self::engine();
    restarted.load(engine.save());
    assert_eq!(restarted.begin(&pr).unwrap(), Step::Done(vec![]));

    let next_task = signal(SignalKind::UserMessage {
        text: "A new HPDP task".into(),
        first_in_context: false,
        skills: vec![],
        tools: vec![],
        model: None,
        code_mode: vec![],
    });
    restarted.begin(&next_task).unwrap();
    assert_eq!(restarted.begin(&pr).unwrap(), Step::Done(vec![]));
    assert_eq!(restarted.begin(&edit).unwrap(), Step::Done(vec![]));
    assert!(matches!(restarted.begin(&pr).unwrap(), Step::Ask { .. }));
}

#[test]
fn a_one_step_effect_survives_another_skill_moving_to_its_second_step() {
    let workspace = Workspace::new();
    std::fs::write(workspace.skill("changelog.json"), ONE_STEP).unwrap();
    let mut engine = engine();
    engine.begin(&edit(&workspace.project())).unwrap();

    let Step::Ask { questions, .. } = engine.begin(&turn_end(&workspace.project(), "")).unwrap()
    else {
        panic!("expected both first steps")
    };
    let answers = questions
        .iter()
        .map(|question| answer(&question.id, 0.95))
        .collect();
    let Step::Ask { questions, .. } = engine.finish(Ok(answers), 1) else {
        panic!("expected readiness")
    };
    let Step::Done(effects) = engine.finish(Ok(vec![answer(&questions[0].id, 0.95)]), 1) else {
        panic!("expected effects")
    };
    let labels: Vec<&str> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Context { label, .. } => Some(label.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(labels, vec!["Changelog", "HPDP PR"]);
}

#[test]
fn edits_from_another_workspace_do_not_activate_a_project_skill() {
    let workspace = Workspace::new();
    let mut engine = engine();

    assert_eq!(
        engine.begin(&edit(&workspace.sibling())).unwrap(),
        Step::Done(vec![])
    );
    assert_eq!(
        engine
            .begin(&turn_end(&workspace.project(), "open a PR"))
            .unwrap(),
        Step::Done(vec![])
    );
}

#[test]
fn invalid_and_duplicate_contracts_are_rejected() {
    let workspace = Workspace::new();
    let project = workspace.project();
    let file = workspace.skill("pr.json");

    std::fs::write(
        &file,
        CONTRACT.replace("\"once\":true", "\"once\":true,\"unknown\":0"),
    )
    .unwrap();
    assert!(load(project.to_str().unwrap()).is_err());

    std::fs::write(&file, CONTRACT).unwrap();
    std::fs::write(workspace.skill("duplicate.json"), CONTRACT).unwrap();
    assert!(load(project.to_str().unwrap()).is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_project_contract_cannot_read_outside_the_worktree() {
    let workspace = Workspace::new();
    let outside = workspace.0.join("outside.json");
    std::fs::write(&outside, CONTRACT).unwrap();
    std::os::unix::fs::symlink(&outside, workspace.skill("outside.json")).unwrap();

    assert!(load(workspace.project().to_str().unwrap()).is_err());
}
