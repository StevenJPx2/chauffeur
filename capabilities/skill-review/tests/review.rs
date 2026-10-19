//! A skill review through the real engine, with fake judges.

use std::sync::{Arc, Mutex};

use chauffeur_capability_skill_review::{SkillReview, SkillReviewConfig};
use chauffeur_core::system_two::MAX_REASONING_INPUT_BYTES;
use chauffeur_core::{
    Answer, AnswerValue, Effect, Engine, Judging, Question, Reasoned, Reasoning, ReasoningFailure,
    Signal, SignalKind, SkillFile, SkillSafety, SystemOne, SystemOneError, SystemTwo,
};
use serde_json::json;

/// Answers P(safe) = the given value, or fails; keeps every question.
struct One {
    answer: Result<f32, ()>,
    seen: Arc<Mutex<Vec<Question>>>,
}

impl SystemOne for One {
    fn name(&self) -> &str {
        "one"
    }

    fn ask(&mut self, _: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        self.seen.lock().unwrap().extend_from_slice(questions);
        let p = self.answer.map_err(|()| SystemOneError("down".into()))?;

        Ok(questions
            .iter()
            .map(|question| Answer {
                id: question.id.clone(),
                value: AnswerValue::Noul(p),
                confidence: None,
            })
            .collect())
    }
}

struct Two {
    reply: Reasoned,
    seen: Arc<Mutex<Vec<Reasoning>>>,
}

impl SystemTwo for Two {
    fn name(&self) -> &str {
        "two"
    }

    fn reason(&mut self, request: &Reasoning) -> Reasoned {
        self.seen.lock().unwrap().push(request.clone());
        self.reply.clone()
    }

    fn metadata(&mut self, _: &Reasoning) -> chauffeur_core::AttemptMetadata {
        chauffeur_core::AttemptMetadata::default()
    }
}

struct Run {
    effects: Vec<Effect>,
    asked: Vec<Question>,
    reviewed: Vec<Reasoning>,
}

fn run(
    one: Result<f32, ()>,
    two: Option<Reasoned>,
    config: SkillReviewConfig,
    signal: &Signal,
) -> Run {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let reviewed = Arc::new(Mutex::new(Vec::new()));
    let engine = Engine::new(
        Box::new(One {
            answer: one,
            seen: Arc::clone(&asked),
        }),
        vec![Box::new(Judging::new(
            SkillReview::default().with_config(config),
        ))],
    )
    .unwrap();
    let mut engine = match two {
        Some(reply) => engine.with_system_two(Box::new(Two {
            reply,
            seen: Arc::clone(&reviewed),
        })),
        None => engine,
    };
    let effects = engine.ingest(signal).unwrap();

    Run {
        effects,
        asked: asked.lock().unwrap().clone(),
        reviewed: reviewed.lock().unwrap().clone(),
    }
}

fn review(instructions: &str, files: Vec<SkillFile>) -> Signal {
    Signal {
        agent_id: "skill-review:o/r/s".into(),
        at: 1,
        kind: SignalKind::SkillReview {
            name: "deploy".into(),
            source: "https://github.com/o/r.git skills/deploy".into(),
            provenance: "Author: Someone. License: MIT. Commit: abc.".into(),
            instructions: instructions.into(),
            files,
        },
    }
}

fn file(path: &str, content: Option<&str>) -> SkillFile {
    SkillFile {
        path: path.into(),
        content: content.map(Into::into),
    }
}

fn plain() -> Signal {
    review(
        "---\nname: deploy\ndescription: Deploy with the project's script\n---\nRun `npm run deploy`.",
        vec![file("scripts/check.sh", Some("npm test"))],
    )
}

/// The one verdict a review settles on.
fn verdict(effects: &[Effect]) -> (SkillSafety, String, String, Vec<String>) {
    match effects {
        [
            Effect::SkillVerdict {
                agent_id,
                safety,
                judge,
                reason,
                concerns,
            },
        ] => {
            assert_eq!(agent_id, "skill-review:o/r/s");
            (*safety, judge.clone(), reason.clone(), concerns.clone())
        }
        other => panic!("{other:?}"),
    }
}

fn reviewed(verdict: &str, concerns: serde_json::Value) -> Option<Reasoned> {
    Some(Ok(
        json!({ "verdict": verdict, "concerns": concerns, "reason": "Read every file." }),
    ))
}

#[test]
fn a_confident_screen_settles_without_a_review() {
    let safe = run(
        Ok(0.95),
        reviewed("unsafe", json!([])),
        SkillReviewConfig::default(),
        &plain(),
    );
    let (safety, judge, _, _) = verdict(&safe.effects);

    assert_eq!((safety, judge.as_str()), (SkillSafety::Safe, "system_one"));
    assert!(safe.reviewed.is_empty());
    // System One read the whole skill, files included.
    assert!(safe.asked[0].instructions.contains("npm run deploy"));
    assert!(
        safe.asked[0]
            .instructions
            .contains("<file path=\"scripts/check.sh\">\nnpm test")
    );

    let unsafe_ = run(Ok(0.05), None, SkillReviewConfig::default(), &plain());

    assert_eq!(verdict(&unsafe_.effects).0, SkillSafety::Unsafe);
    assert!(unsafe_.reviewed.is_empty());
}

#[test]
fn an_unsure_or_failed_screen_goes_to_a_review_that_must_cite_its_concerns() {
    for one in [Ok(0.6), Err(())] {
        let unsafe_ = run(
            one,
            reviewed(
                "unsafe",
                json!([{"cite": "file-1", "reason": "Uploads ~/.ssh to a server."}]),
            ),
            SkillReviewConfig::default(),
            &plain(),
        );
        let (safety, judge, _, concerns) = verdict(&unsafe_.effects);

        assert_eq!(
            (safety, judge.as_str()),
            (SkillSafety::Unsafe, "system_two")
        );
        assert_eq!(concerns, ["file-1: Uploads ~/.ssh to a server."]);

        let request = &unsafe_.reviewed[0];
        let ids: Vec<&str> = request
            .evidence
            .iter()
            .map(|piece| piece.id.as_str())
            .collect();

        assert_eq!(request.profile, "skill-review");
        assert!(!request.activity);
        assert_eq!(ids, ["provenance", "skill_md", "file-1"]);
    }

    let safe = run(
        Ok(0.6),
        reviewed("safe", json!([])),
        SkillReviewConfig::default(),
        &plain(),
    );

    assert_eq!(verdict(&safe.effects).0, SkillSafety::Safe);

    // Unsafe without a cited concern, and an unavailable review, decide nothing.
    for reply in [
        reviewed("unsafe", json!([])),
        Some(Err(ReasoningFailure::Unavailable(
            "System Two is off".into(),
        ))),
    ] {
        let (safety, judge, reason, _) =
            verdict(&run(Ok(0.6), reply, SkillReviewConfig::default(), &plain()).effects);

        assert_eq!(
            (safety, judge.as_str()),
            (SkillSafety::Undetermined, ""),
            "{reason}"
        );
    }

    let (safety, _, reason, _) = verdict(
        &run(
            Ok(0.6),
            reviewed("insufficient_evidence", json!([])),
            SkillReviewConfig::default(),
            &plain(),
        )
        .effects,
    );

    assert_eq!(
        (safety, reason.as_str()),
        (SkillSafety::Undetermined, "Read every file.")
    );
}

#[test]
fn system_one_cannot_clear_what_it_did_not_see() {
    let binary = review("Run the bundled tool.", vec![file("bin/tool", None)]);
    let run = run(
        Ok(0.95),
        reviewed("insufficient_evidence", json!([])),
        SkillReviewConfig::default(),
        &binary,
    );

    assert_eq!(verdict(&run.effects).0, SkillSafety::Undetermined);
    assert!(
        run.asked[0]
            .instructions
            .contains("Also carries, not shown: bin/tool")
    );
    assert_eq!(
        run.reviewed[0].evidence.last().unwrap().text,
        "Files the skill carries that are not shown: bin/tool"
    );
}

#[test]
fn without_a_review_an_unsure_screen_is_undetermined() {
    let mut config = SkillReviewConfig::default();

    config.review.enabled = false;

    let run = run(Ok(0.6), reviewed("safe", json!([])), config, &plain());
    let (safety, _, reason, _) = verdict(&run.effects);

    assert_eq!(safety, SkillSafety::Undetermined);
    assert!(reason.contains("unsure"), "{reason}");
    assert!(run.reviewed.is_empty());
}

#[test]
fn a_long_skill_is_cut_to_the_review_budget_and_says_so() {
    let long = "Follow these steps carefully. ".repeat(2_000);
    let big = "echo ok\n".repeat(4_000);
    let signal = review(
        &long,
        vec![
            file("scripts/a.sh", Some("echo a")),
            file("scripts/big.sh", Some(&big)),
        ],
    );
    let run = run(
        Ok(0.95),
        reviewed("insufficient_evidence", json!([])),
        SkillReviewConfig::default(),
        &signal,
    );
    let request = &run.reviewed[0];
    let skill_md = &request
        .evidence
        .iter()
        .find(|piece| piece.id == "skill_md")
        .unwrap()
        .text;

    // Too long for System One to clear, so it was reviewed.
    assert_eq!(verdict(&run.effects).0, SkillSafety::Undetermined);
    assert!(request.input_bytes() <= MAX_REASONING_INPUT_BYTES);
    assert!(skill_md.ends_with("[cut: SKILL.md continues]"));
    assert!(
        request
            .evidence
            .last()
            .unwrap()
            .text
            .contains("scripts/big.sh")
    );
}

#[test]
fn other_signals_and_empty_skills_are_not_reviewed() {
    let mut empty = plain();

    if let SignalKind::SkillReview { instructions, .. } = &mut empty.kind {
        instructions.clear();
    }

    let asked = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new(
        Box::new(One {
            answer: Ok(0.9),
            seen: Arc::clone(&asked),
        }),
        vec![Box::new(Judging::new(SkillReview::default()))],
    )
    .unwrap();

    assert!(engine.ingest(&empty).is_err());

    let turn = Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::TurnEnd {
            workspace: String::new(),
            subagent: false,
            user_request: "ship".into(),
            summary: "done".into(),
            todos: Vec::new(),
        },
    };

    assert!(engine.ingest(&turn).unwrap().is_empty());
    assert!(asked.lock().unwrap().is_empty());
}
