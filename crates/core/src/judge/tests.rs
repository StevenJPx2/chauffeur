use super::strategy::{self, Candidate, Chained};
use super::{Judge, NONE, Rule};
use crate::system_one::{Answer, AnswerValue, ChoiceOption, Question, QuestionKind};

fn noul_question(id: &str) -> Question {
    Question {
        id: id.into(),
        instructions: format!("is {id} so?"),
        kind: QuestionKind::Noul,
    }
}

fn choice_question(id: &str, options: &[&str]) -> Question {
    Question {
        id: id.into(),
        instructions: "which?".into(),
        kind: QuestionKind::Choice {
            options: options
                .iter()
                .map(|value| ChoiceOption {
                    value: (*value).into(),
                    description: String::new(),
                })
                .collect(),
        },
    }
}

fn noul(id: &str, p: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

fn picked(id: &str, value: &str, confidence: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Choice(value.into()),
        confidence: Some(confidence),
    }
}

/// Drive `judge` through `rounds` (`None`: the call failed), then settle.
/// Returns its value and the question IDs each round asked.
fn run<T: Send + 'static>(
    judge: Judge<T>,
    rounds: Vec<Option<Vec<Answer>>>,
) -> (T, Vec<Vec<String>>) {
    let mut judge = judge;
    let mut asked = Vec::new();

    for answers in rounds {
        if judge.questions().is_empty() {
            break;
        }

        asked.push(
            judge
                .questions()
                .iter()
                .map(|question| question.id.clone())
                .collect(),
        );
        judge = judge.step(answers.as_deref());
    }

    (judge.settle(), asked)
}

const YES: Rule = Rule::yes(0.7, 0.4);

// Primitives.

#[test]
fn done_asks_nothing_and_ask_reads_its_answer_or_a_failure() {
    let (value, asked) = run(Judge::done(7), vec![Some(vec![])]);

    assert_eq!((value, asked.len()), (7, 0));
    assert!(
        run(
            strategy::single(noul_question("a"), YES),
            vec![Some(vec![noul("a", 0.9)])]
        )
        .0
    );
    assert!(!run(strategy::single(noul_question("a"), YES), vec![None]).0);
}

#[test]
fn then_asks_its_continuation_in_the_following_round() {
    let judge = strategy::single(noul_question("first"), YES).then(|held| {
        if held {
            strategy::single(noul_question("second"), YES)
        } else {
            Judge::done(false)
        }
    });
    let (held, asked) = run(
        judge,
        vec![
            Some(vec![noul("first", 0.9)]),
            Some(vec![noul("second", 0.9)]),
        ],
    );

    assert!(held);
    assert_eq!(asked, [vec!["first"], vec!["second"]]);
}

#[test]
fn all_asks_side_by_side_and_finishes_in_order_when_the_slowest_does() {
    let quick = strategy::single(noul_question("quick"), YES);
    let slow = strategy::single(noul_question("slow-1"), YES)
        .then(|_| strategy::single(noul_question("slow-2"), YES));
    let (values, asked) = run(
        Judge::all(vec![quick, slow, Judge::done(true)]),
        vec![
            Some(vec![noul("quick", 0.2), noul("slow-1", 0.9)]),
            Some(vec![noul("slow-2", 0.9)]),
        ],
    );

    assert_eq!(values, [false, true, true]);
    assert_eq!(asked, [vec!["quick", "slow-1"], vec!["slow-2"]]);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "distinct question IDs")]
fn judges_run_together_cannot_share_a_question_id() {
    let _ = Judge::all(vec![
        strategy::single(noul_question("same"), YES),
        strategy::single(noul_question("same"), YES),
    ]);
}

#[test]
fn zip_runs_judges_of_different_values_side_by_side() {
    let count = strategy::single(noul_question("group"), YES).map(usize::from);
    let name = Judge::ask(choice_question("pick", &["safari", NONE]), |answer| {
        Rule::pick(0.4).chosen(answer)
    });
    let ((count, name), asked) = run(
        count.zip(name),
        vec![Some(vec![
            noul("group", 0.9),
            picked("pick", "safari", 0.9),
        ])],
    );

    assert_eq!((count, name.as_deref()), (1, Some("safari")));
    assert_eq!(asked, [vec!["group", "pick"]]);
}

#[test]
fn unless_failed_tells_a_failed_call_from_rules_that_refused() {
    let judge = || strategy::fan_out([candidate("slack", YES)]).unless_failed();

    assert_eq!(
        run(judge(), vec![Some(vec![noul("slack", 0.1)])]).0,
        Some(Vec::new())
    );
    assert_eq!(run(judge(), vec![None]).0, None);
    assert_eq!(run(Judge::done(3).unless_failed(), vec![]).0, Some(3));
}

#[test]
fn a_judge_still_asking_when_rounds_run_out_settles_as_failed() {
    let judge = strategy::single(noul_question("first"), YES)
        .then(|_| strategy::single(noul_question("second"), YES));
    let (held, asked) = run(judge, vec![Some(vec![noul("first", 0.9)])]);

    assert!(!held);
    assert_eq!(asked.len(), 1);
}

// Rules.

#[test]
fn rules_read_yes_unless_no_and_picks() {
    let unless = Rule::unless_no(0.3, 0.4);
    let no = Rule::no(0.3, 0.4);
    let pick = Rule::pick(0.4);

    assert!(no.holds(Some(&noul("a", 0.1))) && !no.holds(Some(&noul("a", 0.5))));
    assert!(!no.holds(None), "a failed call is not a confident no");

    assert!(YES.holds(Some(&noul("a", 0.7))) && !YES.holds(Some(&noul("a", 0.6))));
    assert!(unless.holds(Some(&noul("a", 0.55))) && unless.holds(Some(&noul("a", 0.35))));
    assert!(!unless.holds(Some(&noul("a", 0.04))));
    assert!(
        !YES.holds(None) && !unless.holds(None),
        "a failed call never holds"
    );
    assert_eq!(
        pick.chosen(Some(&picked("p", "slack", 0.9))),
        Some("slack".into())
    );
    assert_eq!(pick.chosen(Some(&picked("p", NONE, 0.9))), None);
    assert_eq!(pick.chosen(Some(&picked("p", "slack", 0.2))), None);
    assert_eq!(pick.chosen(None), None);
}

// Strategies.

fn candidate(key: &str, rule: Rule) -> Candidate<String> {
    Candidate {
        key: key.into(),
        question: noul_question(key),
        rule,
    }
}

#[test]
fn fan_out_admits_every_candidate_that_holds_most_likely_first_in_one_round() {
    let judge = strategy::fan_out([
        candidate("slack", YES),
        candidate("jira", YES),
        candidate("rust", YES),
        candidate("project", Rule::unless_no(0.3, 0.4)),
    ]);
    let (admitted, asked) = run(
        judge,
        vec![Some(vec![
            noul("slack", 0.85),
            noul("jira", 0.91),
            noul("rust", 0.6),
            noul("project", 0.55),
        ])],
    );

    assert_eq!(admitted, ["jira", "slack", "project"]);
    assert_eq!(asked.len(), 1);
    assert!(
        run(strategy::fan_out([candidate("slack", YES)]), vec![None])
            .0
            .is_empty()
    );
}

#[test]
fn chain_asks_a_step_only_after_the_previous_held() {
    let chained = |key: &str| Chained {
        key: key.to_string(),
        steps: vec![
            (noul_question(&format!("{key}/intent")), YES),
            (noul_question(&format!("{key}/ready")), YES),
        ],
    };
    let one_step = Chained {
        key: "changelog".to_string(),
        steps: vec![(noul_question("changelog/notable"), YES)],
    };
    let (admitted, asked) = run(
        strategy::chain([chained("pr"), chained("verify"), one_step]),
        vec![
            Some(vec![
                noul("pr/intent", 0.9),
                noul("verify/intent", 0.1),
                noul("changelog/notable", 0.9),
            ]),
            Some(vec![noul("pr/ready", 0.9)]),
        ],
    );

    assert_eq!(admitted, ["pr", "changelog"]);
    assert_eq!(
        asked,
        [
            vec!["pr/intent", "verify/intent", "changelog/notable"],
            vec!["pr/ready"]
        ]
    );
}

#[test]
fn a_failed_second_round_keeps_what_the_first_admitted() {
    let two = Chained {
        key: "pr".to_string(),
        steps: vec![
            (noul_question("pr/intent"), YES),
            (noul_question("pr/ready"), YES),
        ],
    };
    let one = Chained {
        key: "changelog".to_string(),
        steps: vec![(noul_question("changelog/notable"), YES)],
    };
    let (admitted, _) = run(
        strategy::chain([two, one]),
        vec![
            Some(vec![noul("pr/intent", 0.9), noul("changelog/notable", 0.9)]),
            None,
        ],
    );

    assert_eq!(admitted, ["changelog"]);
}

#[test]
fn a_pick_then_confirm_is_composed_from_the_primitives() {
    let recover = || {
        Judge::ask(
            choice_question("choose", &["browser_open", NONE]),
            |answer| Rule::pick(0.4).chosen(answer),
        )
        .then(|tool| match tool {
            Some(tool) => {
                let confirm = noul_question(&format!("confirm/{tool}"));

                Judge::ask(confirm, move |answer| YES.holds(answer).then_some(tool))
            }
            None => Judge::done(None),
        })
    };
    let (revealed, asked) = run(
        recover(),
        vec![
            Some(vec![picked("choose", "browser_open", 0.9)]),
            Some(vec![noul("confirm/browser_open", 0.9)]),
        ],
    );

    assert_eq!(revealed.as_deref(), Some("browser_open"));
    assert_eq!(asked, [vec!["choose"], vec!["confirm/browser_open"]]);
    assert_eq!(
        run(recover(), vec![Some(vec![picked("choose", NONE, 0.9)])]).0,
        None
    );
}
