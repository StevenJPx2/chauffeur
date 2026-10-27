use std::collections::BTreeMap;

use chauffeur_core::Failure;
use chauffeur_judges::{
    Answer, AnsweredBy, Confidence, ConfidenceError, Label, Milli, Outcomes, Probabilities,
    Question, QuestionError, Settled,
};

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: every caller passes a valid probability"
)]
const fn milli(thousandths: u16) -> Milli {
    Milli::new(thousandths).unwrap()
}

#[expect(
    clippy::unwrap_used,
    reason = "a test helper: every caller passes a valid reply"
)]
fn settle(question: &Question, reply: &[(&str, f64)], confidence: &Confidence) -> Settled {
    let p = Probabilities::read(question.outcomes(), reply.iter().copied()).unwrap();
    Answer::settle(question, p, confidence, by())
        .outcome()
        .clone()
}

fn by() -> AnsweredBy {
    AnsweredBy {
        model: "jev".into(),
        placeholder: false,
    }
}

fn unsure_or(value: &str) -> Settled {
    if value == "unsure" {
        Settled::Unsure
    } else {
        Settled::Outcome(value.into())
    }
}

#[test]
fn a_single_bar_settles_the_commit_examples() {
    let question = Question::noul(
        "Does this commit message break Conventional Commits?",
        ["commit"],
    )
    .unwrap();
    let bar = Confidence::Bar(milli(700));

    for (yes, no, expected) in [
        (0.94, 0.06, "yes"),
        (0.06, 0.94, "no"),
        (0.58, 0.42, "unsure"),
        (0.7, 0.3, "yes"),
    ] {
        assert_eq!(
            settle(&question, &[("yes", yes), ("no", no)], &bar),
            unsure_or(expected),
            "yes {yes}"
        );
    }
}

#[test]
fn per_outcome_bars_settle_the_harm_examples() {
    let question =
        Question::noul("Could this command cause irreversible harm?", ["command"]).unwrap();
    let bars = Confidence::PerOutcome(BTreeMap::from([
        ("yes".into(), milli(600)),
        ("no".into(), milli(950)),
    ]));
    bars.check(question.outcomes()).unwrap();

    for (yes, no, expected) in [
        (0.71, 0.29, "yes"),
        (0.02, 0.98, "no"),
        (0.12, 0.88, "unsure"),
    ] {
        assert_eq!(
            settle(&question, &[("yes", yes), ("no", no)], &bars),
            unsure_or(expected),
            "yes {yes}"
        );
    }
}

#[test]
fn with_no_bar_the_likeliest_settles_and_a_tie_is_unsure() {
    let question = Question::noul("Is this a question?", ["text"]).unwrap();

    assert_eq!(
        settle(
            &question,
            &[("yes", 0.51), ("no", 0.49)],
            &Confidence::Likeliest
        ),
        unsure_or("yes")
    );
    assert_eq!(
        settle(
            &question,
            &[("yes", 0.5), ("no", 0.5)],
            &Confidence::Likeliest
        ),
        Settled::Unsure
    );
}

#[test]
fn level_is_the_weighted_position_on_the_scale() {
    let outcomes = Outcomes::new(["low", "medium", "high"]).unwrap();
    let question = Question::new("How hard is the user's request?", outcomes, ["prompt"]).unwrap();
    let p = Probabilities::read(
        question.outcomes(),
        [("low", 0.12), ("medium", 0.45), ("high", 0.43)],
    )
    .unwrap();

    let answer = Answer::settle(&question, p, &Confidence::Bar(milli(700)), by());

    assert_eq!(answer.outcome(), &Settled::Unsure);
    assert!((answer.level() - 1.31).abs() < 1e-9);
    assert_eq!(
        answer
            .p()
            .map(|(value, p)| (value, p.get()))
            .collect::<Vec<_>>(),
        [("low", 120), ("medium", 450), ("high", 430)]
    );
}

#[test]
fn a_reply_that_is_not_an_answer_is_invalid() {
    let outcomes = Outcomes::new(["yes", "no"]).unwrap();
    let invalid = [
        vec![("yes", 0.9)],
        vec![("yes", 0.9), ("no", 0.1), ("maybe", 0.0)],
        vec![("yes", 0.9), ("yes", 0.1)],
        vec![("yes", 1.2), ("no", 0.1)],
        vec![("yes", f64::NAN), ("no", 0.1)],
        vec![("yes", 0.0), ("no", 0.0)],
    ];

    for reply in invalid {
        assert_eq!(
            Probabilities::read(&outcomes, reply.clone()),
            Err(Failure::Invalid),
            "{reply:?}"
        );
    }
}

#[test]
fn questions_refuse_bad_outcomes_and_blank_instructions() {
    let sixty_five: Vec<String> = (0..65).map(|n| format!("o{n}")).collect();
    let cases = [
        (vec!["only".to_owned()], QuestionError::OutcomeCount(1)),
        (sixty_five, QuestionError::OutcomeCount(65)),
        (
            vec!["a".into(), "a".into()],
            QuestionError::Duplicate("a".into()),
        ),
        (
            vec!["a".into(), "unsure".into()],
            QuestionError::Reserved("unsure".into()),
        ),
        (
            vec!["failed".into(), "a".into()],
            QuestionError::Reserved("failed".into()),
        ),
        (vec!["a".into(), " ".into()], QuestionError::BlankOutcome),
    ];
    for (labels, error) in cases {
        assert_eq!(Outcomes::new(labels.iter().map(String::as_str)), Err(error));
    }

    assert_eq!(
        Question::noul("  ", ["x"]),
        Err(QuestionError::NoInstructions)
    );
    let described = Outcomes::new([
        Label::new("act"),
        Label::described("ask-user", "Unclear; the user decides"),
    ])
    .unwrap();
    assert_eq!(
        described.get(1).and_then(Label::description),
        Some("Unclear; the user decides")
    );
}

#[test]
fn a_per_outcome_confidence_must_cover_exactly_the_outcomes() {
    let outcomes = Outcomes::new(["yes", "no"]).unwrap();
    let missing = Confidence::PerOutcome(BTreeMap::from([("yes".into(), milli(600))]));
    let extra = Confidence::PerOutcome(BTreeMap::from([
        ("yes".into(), milli(600)),
        ("no".into(), milli(600)),
        ("maybe".into(), milli(600)),
    ]));

    assert_eq!(
        missing.check(&outcomes),
        Err(ConfidenceError::Missing("no".into()))
    );
    assert_eq!(
        extra.check(&outcomes),
        Err(ConfidenceError::Unknown("maybe".into()))
    );
    assert_eq!(Confidence::Bar(milli(700)).check(&outcomes), Ok(()));
}

#[test]
fn probabilities_round_to_the_nearest_thousandth() {
    assert_eq!(Milli::from_f64(0.9994).map(Milli::get), Some(999));
    assert_eq!(Milli::from_f64(0.9996).map(Milli::get), Some(1000));
    assert_eq!(Milli::from_f64(-0.001), None);
    assert_eq!(Milli::new(1001), None);
    assert!((milli(250).to_f64() - 0.25).abs() < 1e-12);
}
