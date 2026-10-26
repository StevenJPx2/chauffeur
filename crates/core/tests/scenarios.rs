//! End-to-end runs of the strategies the playground offers, driven through the
//! public API exactly as a runtime would.

use std::cell::{Cell, RefCell};

use chauffeur_core::{
    AnswerError, Event, Gate, Judge, Next, Outcome, Panel, Plan, PlanError, Resolution, Step,
    Verdict, When, all, ask,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum A {
    Yes,
    No,
    Unsure,
}

use A::{No, Unsure, Yes};

/// The judge every hand-driven scenario names; `drive` answers for it.
const PERSON: &str = "person";

const fn value(answer: A) -> Outcome<A> {
    Outcome::Value(answer)
}

fn step(id: &str) -> Step<&'static str, A> {
    Step::new(id, PERSON, "?")
}

fn by_hand(id: &str) -> Plan<&'static str, A> {
    ask(id, PERSON, "?")
}

/// Run `plan` by hand, answering each asked step with `answer(id)`. Returns
/// the verdict, every event, and the steps each round asked.
#[allow(
    clippy::expect_used,
    reason = "a scenario fails loudly on an invalid plan or reply"
)]
fn drive(
    plan: Plan<&str, A>,
    answer: impl Fn(&str) -> Outcome<A>,
) -> (Verdict<A>, Vec<Event<A>>, Vec<Vec<String>>) {
    let mut transition = plan.start().expect("valid plan");
    let mut events = Vec::new();
    let mut rounds = Vec::new();
    loop {
        events.append(&mut transition.events);
        match transition.next {
            Next::Done(verdict) => return (verdict, events, rounds),
            Next::Ask(round) => {
                let asked: Vec<String> = round.questions().map(|q| q.step.to_owned()).collect();
                let reply: Vec<_> = asked.iter().map(|id| (id.clone(), answer(id))).collect();
                rounds.push(asked);
                transition = round.answer(reply).expect("complete reply");
            }
        }
    }
}

const fn settled(answer: A) -> Resolution<A> {
    Resolution::Settled(value(answer))
}

#[test]
fn single_step_settles_in_one_round() {
    let (verdict, events, rounds) = drive(by_hand("q"), |_| value(Yes));

    assert_eq!(rounds, [["q"]]);
    assert_eq!(verdict.get("q"), Some(&settled(Yes)));
    assert_eq!(
        events,
        [
            Event::Started { steps: 1 },
            Event::RoundReady {
                round: 1,
                steps: vec!["q".into()]
            },
            Event::StepSettled {
                round: 1,
                step: "q".into(),
                outcome: value(Yes)
            },
            Event::Finished { rounds: 1 },
        ]
    );
}

#[test]
fn chain_runs_one_step_per_round() {
    let plan = by_hand("a")
        .then_if_any(value(Yes), by_hand("b"))
        .then_if_any(value(Yes), by_hand("c"));

    let (verdict, _, rounds) = drive(plan, |_| value(Yes));

    assert_eq!(rounds, [["a"], ["b"], ["c"]]);
    assert_eq!(verdict.get("c"), Some(&settled(Yes)));
}

#[test]
fn chain_skip_propagates_to_every_later_step() {
    let plan = by_hand("a")
        .then_if_any(value(Yes), by_hand("b"))
        .then_if_any(value(Yes), by_hand("c"));

    let (verdict, events, rounds) = drive(plan, |_| value(No));

    assert_eq!(rounds, [["a"]]);
    assert_eq!(verdict.get("b"), Some(&Resolution::Skipped));
    assert_eq!(verdict.get("c"), Some(&Resolution::Skipped));
    let skipped: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            Event::StepSkipped { step } => Some(step.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(skipped, ["b", "c"]);
}

#[test]
fn fan_out_asks_every_step_in_one_round() {
    let plan = all([by_hand("x"), by_hand("y"), by_hand("z")]);

    let (_, _, rounds) = drive(plan, |_| value(No));

    assert_eq!(rounds, [["x", "y", "z"]]);
}

fn screen_review() -> Plan<&'static str, A> {
    by_hand("screen").then_on(
        When::any([value(Unsure), Outcome::Failed]),
        by_hand("review"),
    )
}

#[test]
fn screen_review_skips_review_when_screen_is_confident() {
    let (verdict, _, rounds) = drive(screen_review(), |_| value(No));

    assert_eq!(rounds, [["screen"]]);
    assert_eq!(verdict.get("review"), Some(&Resolution::Skipped));
}

#[test]
fn screen_review_reviews_an_unsure_or_failed_screen() {
    for screen in [value(Unsure), Outcome::Failed] {
        let reply = |id: &str| {
            if id == "screen" {
                screen.clone()
            } else {
                value(Yes)
            }
        };

        let (verdict, _, rounds) = drive(screen_review(), reply);

        assert_eq!(rounds, [["screen"], ["review"]]);
        assert_eq!(verdict.get("review"), Some(&settled(Yes)));
    }
}

#[test]
fn fallback_runs_only_after_a_failure() {
    let plan = by_hand("primary").then_if_any(Outcome::Failed, by_hand("fallback"));

    let (verdict, _, rounds) = drive(plan.clone(), |_| value(Yes));
    assert_eq!(rounds, [["primary"]]);
    assert_eq!(verdict.get("fallback"), Some(&Resolution::Skipped));

    let mut round = plan.start().expect("valid").next.ask().expect("round 1");
    round = round.fail().next.ask().expect("fallback round");
    assert_eq!(
        round.questions().map(|q| q.step).collect::<Vec<_>>(),
        ["fallback"]
    );
}

#[test]
fn fan_out_review_runs_once_any_candidate_says_yes() {
    let plan = all([by_hand("slack"), by_hand("jira"), by_hand("memory")])
        .then_if_any(value(Yes), by_hand("review"));
    let reply = |id: &str| match id {
        "jira" | "review" => value(Yes),
        _ => value(No),
    };

    let (verdict, _, rounds) = drive(plan.clone(), reply);
    assert_eq!(rounds, [vec!["slack", "jira", "memory"], vec!["review"]]);
    assert_eq!(verdict.get("review"), Some(&settled(Yes)));

    let (verdict, _, _) = drive(plan, |_| value(No));
    assert_eq!(verdict.get("review"), Some(&Resolution::Skipped));
}

#[test]
fn all_gate_skips_on_the_first_mismatch() {
    let plan = Plan::new([
        step("a"),
        step("b"),
        step("both").gated(Gate::all(["a", "b"], [value(Yes)])),
    ]);
    let reply = |id: &str| if id == "a" { value(Yes) } else { value(No) };

    let (verdict, _, rounds) = drive(plan, reply);

    assert_eq!(rounds, [["a", "b"]]);
    assert_eq!(verdict.get("both"), Some(&Resolution::Skipped));
}

#[test]
fn every_gate_on_a_step_must_pass() {
    let plan = Plan::new([
        step("a"),
        step("b"),
        step("c")
            .gated(Gate::any(["a"], [value(Yes)]))
            .gated(Gate::any(["b"], [value(Yes)])),
    ]);
    let reply = |id: &str| if id == "a" { value(Yes) } else { value(No) };

    let (verdict, _, _) = drive(plan, reply);

    assert_eq!(verdict.get("c"), Some(&Resolution::Skipped));
}

#[test]
fn invalid_plans_are_rejected_before_any_round() {
    let cases: [(Plan<&str, A>, PlanError); 5] = [
        (Plan::new([]), PlanError::Empty),
        (
            all([by_hand("a"), by_hand("a")]),
            PlanError::DuplicateStep("a".into()),
        ),
        (
            Plan::new([step("a").gated(Gate::any(["a"], [value(Yes)]))]),
            PlanError::NotPreceding {
                step: "a".into(),
                source: "a".into(),
            },
        ),
        (
            Plan::new([
                step("a"),
                step("b").gated(Gate::any(Vec::<String>::new(), [value(Yes)])),
            ]),
            PlanError::EmptySources { step: "b".into() },
        ),
        (
            Plan::new([step("a"), step("b").gated(Gate::any(["a"], []))]),
            PlanError::EmptyOutcomes { step: "b".into() },
        ),
    ];

    for (plan, expected) in cases {
        assert_eq!(plan.start().err(), Some(expected));
    }
}

#[test]
fn a_rejected_reply_returns_the_unchanged_round() {
    let plan = all([by_hand("a"), by_hand("b")]).then_if_any(value(Yes), by_hand("c"));
    let round = plan.start().expect("valid").next.ask().expect("round 1");
    let cases = [
        (vec![("a", value(Yes))], AnswerError::Missing("b".into())),
        (
            vec![("a", value(Yes)), ("a", value(No)), ("b", value(No))],
            AnswerError::Duplicate("a".into()),
        ),
        (
            vec![("zzz", value(Yes))],
            AnswerError::UnknownStep("zzz".into()),
        ),
        (vec![("c", value(Yes))], AnswerError::NotAsked("c".into())),
    ];

    for (reply, expected) in cases {
        let rejected = round.clone().answer(reply).expect_err("rejected");
        assert_eq!(rejected.error, expected);
        assert_eq!(rejected.round, round);
    }
}

#[test]
fn events_render_for_people() {
    let event: Event<&str> = Event::StepSettled {
        round: 2,
        step: "review".into(),
        outcome: Outcome::Failed,
    };

    assert_eq!(event.to_string(), "round 2: review settled failed");
}

#[test]
fn a_round_reports_which_judge_each_step_asks() {
    let plan: Plan<&str, A> = all([
        ask("rule", "keywords", "?"),
        ask("model", "system-one", "?"),
    ]);
    let round = plan.start().expect("valid").next.ask().expect("round 1");

    let asked: Vec<_> = round.questions().map(|q| (q.step, q.judge)).collect();

    assert_eq!(asked, [("rule", "keywords"), ("model", "system-one")]);
}

/// A judge that records each batch it is asked and answers `answer` to all.
struct Recorder {
    answer: A,
    batches: RefCell<Vec<Vec<&'static str>>>,
}

impl Recorder {
    const fn new(answer: A) -> Self {
        Self {
            answer,
            batches: RefCell::new(Vec::new()),
        }
    }
}

impl Judge<&'static str, A> for Recorder {
    fn judge(&self, request: &&'static str) -> Outcome<A> {
        self.judge_all(&[request]).remove(0)
    }

    fn judge_all(&self, requests: &[&&'static str]) -> Vec<Outcome<A>> {
        self.batches
            .borrow_mut()
            .push(requests.iter().map(|request| **request).collect());
        requests.iter().map(|_| value(self.answer)).collect()
    }
}

#[test]
fn a_rule_escalates_to_a_model_only_when_unsure() {
    let rule = |evidence: &&str| {
        if evidence.contains("TODO") {
            value(Yes)
        } else {
            value(Unsure)
        }
    };
    let model = Recorder::new(No);
    let panel = Panel::new().with("rule", &rule).with("model", &model);
    let plan = |evidence| {
        ask("screen", "rule", evidence).then_if_any(value(Unsure), ask("review", "model", evidence))
    };

    let confident = plan("fix the TODO").run(&panel).expect("valid plan");
    let unsure = plan("tests pass").run(&panel).expect("valid plan");

    assert_eq!(confident.verdict.get("review"), Some(&Resolution::Skipped));
    assert_eq!(unsure.verdict.get("review"), Some(&settled(No)));
    assert_eq!(*model.batches.borrow(), [["tests pass"]]);
    assert_eq!(
        unsure.events.last(),
        Some(&Event::Finished { rounds: 2 }),
        "the run's events end with its finish"
    );
}

#[test]
fn each_judge_answers_its_share_of_a_round_in_one_call() {
    let first = Recorder::new(Yes);
    let second = Recorder::new(No);
    let panel = Panel::new().with("first", &first).with("second", &second);
    let plan = all([
        ask("x", "first", "x?"),
        ask("y", "second", "y?"),
        ask("z", "first", "z?"),
    ]);

    let run = plan.run(&panel).expect("valid plan");

    assert_eq!(*first.batches.borrow(), [["x?", "z?"]]);
    assert_eq!(*second.batches.borrow(), [["y?"]]);
    assert_eq!(run.verdict.get("y"), Some(&settled(No)));
    assert_eq!(run.verdict.get("z"), Some(&settled(Yes)));
}

#[test]
fn a_plan_naming_a_missing_judge_fails_before_any_judge_is_asked() {
    let calls = Cell::new(0_u32);
    let counted = |_: &&str| {
        calls.set(calls.get() + 1);
        value(Yes)
    };
    let panel = Panel::new().with("rule", &counted);
    let plan = ask("screen", "rule", "?").then_if_any(value(Yes), ask("review", "model", "?"));

    let error = plan.run(&panel).expect_err("the panel lacks `model`");

    assert_eq!(
        error,
        PlanError::UnknownJudge {
            step: "review".into(),
            judge: "model".into(),
        }
    );
    assert_eq!(calls.get(), 0);
}

#[test]
fn a_batch_with_the_wrong_number_of_answers_settles_as_failed() {
    struct Silent;
    impl Judge<&'static str, A> for Silent {
        fn judge(&self, _: &&'static str) -> Outcome<A> {
            value(Yes)
        }
        fn judge_all(&self, _: &[&&'static str]) -> Vec<Outcome<A>> {
            Vec::new()
        }
    }
    let panel = Panel::new().with("silent", &Silent);

    let run = all([ask("a", "silent", "?"), ask("b", "silent", "?")])
        .run(&panel)
        .expect("valid plan");

    assert_eq!(
        run.verdict.get("a"),
        Some(&Resolution::Settled(Outcome::Failed))
    );
    assert_eq!(
        run.verdict.get("b"),
        Some(&Resolution::Settled(Outcome::Failed))
    );
}
