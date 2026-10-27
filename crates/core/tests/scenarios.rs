use std::cell::Cell;

use chauffeur_core::{Event, Failure, Judge, Outcome, Panel, Plan, PlanError, Step};

/// Counts its `judge_all` calls and answers each request with itself.
struct Counting {
    calls: Cell<u32>,
}

impl Judge<&'static str, &'static str> for Counting {
    fn judge(&self, request: &&'static str) -> Outcome<&'static str> {
        Outcome::Value(request)
    }

    fn judge_all(&self, requests: &[&&'static str]) -> Vec<Outcome<&'static str>> {
        self.calls.set(self.calls.get().saturating_add(1));
        requests.iter().map(|request| self.judge(request)).collect()
    }
}

/// Always returns one answer, whatever it is asked.
struct Short;

impl Judge<&'static str, &'static str> for Short {
    fn judge(&self, _: &&'static str) -> Outcome<&'static str> {
        Outcome::Value("only")
    }

    fn judge_all(&self, _: &[&&'static str]) -> Vec<Outcome<&'static str>> {
        vec![Outcome::Value("only")]
    }
}

#[test]
fn an_empty_plan_or_a_repeated_id_is_refused() {
    assert_eq!(Plan::<()>::new([]), Err(PlanError::Empty));
    assert_eq!(
        Plan::new([Step::new("a", "j", ()), Step::new("a", "j", ())]),
        Err(PlanError::DuplicateStep("a".into()))
    );
}

#[test]
fn a_judge_the_panel_lacks_is_refused_before_anyone_is_asked() {
    let counting = Counting {
        calls: Cell::new(0),
    };
    let plan = Plan::new([Step::new("a", "known", "x"), Step::new("b", "missing", "y")]).unwrap();
    let panel = Panel::new().with("known", &counting);

    let error = plan.run(&panel).unwrap_err();

    assert_eq!(
        error,
        PlanError::UnknownJudge {
            step: "b".into(),
            judge: "missing".into()
        }
    );
    assert_eq!(
        error.to_string(),
        "step `b` names judge `missing`, which the panel lacks"
    );
    assert_eq!(counting.calls.get(), 0);
}

#[test]
fn each_judge_answers_its_share_in_one_call() {
    let one = Counting {
        calls: Cell::new(0),
    };
    let two = Counting {
        calls: Cell::new(0),
    };
    let plan = Plan::new([
        Step::new("a", "one", "first"),
        Step::new("b", "two", "second"),
        Step::new("c", "one", "third"),
    ])
    .unwrap();
    let panel = Panel::new().with("one", &one).with("two", &two);

    let run = plan.run(&panel).unwrap();

    assert_eq!((one.calls.get(), two.calls.get()), (1, 1));
    let verdict: Vec<_> = run
        .verdict
        .iter()
        .map(|(id, outcome)| (id, outcome.clone()))
        .collect();
    assert_eq!(
        verdict,
        [
            ("a", Outcome::Value("first")),
            ("b", Outcome::Value("second")),
            ("c", Outcome::Value("third"))
        ]
    );
}

#[test]
fn a_judge_added_again_under_its_name_replaces_the_first() {
    let first = |_: &&str| Outcome::Value("first");
    let second = |_: &&str| Outcome::Value("second");
    let plan = Plan::new([Step::new("a", "j", "x")]).unwrap();
    let panel = Panel::new().with("j", &first).with("j", &second);

    let run = plan.run(&panel).unwrap();

    assert_eq!(run.verdict.get("a"), Some(&Outcome::Value("second")));
}

#[test]
fn a_batch_of_the_wrong_length_fails_every_step_in_it() {
    let plan = Plan::new([Step::new("a", "short", "x"), Step::new("b", "short", "y")]).unwrap();
    let panel = Panel::new().with("short", &Short);

    let run = plan.run(&panel).unwrap();

    assert_eq!(
        run.verdict.get("a"),
        Some(&Outcome::Failed(Failure::Invalid))
    );
    assert_eq!(
        run.verdict.get("b"),
        Some(&Outcome::Failed(Failure::Invalid))
    );
}

#[test]
fn a_failure_stays_a_failure_and_events_come_in_order() {
    let timeout = |_: &&str| Outcome::<&str>::Failed(Failure::Timeout);
    let plan = Plan::new([Step::new("a", "slow", "x")]).unwrap();
    let panel = Panel::new().with("slow", &timeout);

    let run = plan.run(&panel).unwrap();

    assert_eq!(
        run.events,
        [
            Event::Started {
                steps: vec!["a".into()]
            },
            Event::RoundReady {
                round: 1,
                steps: vec!["a".into()]
            },
            Event::StepSettled {
                step: "a".into(),
                outcome: Outcome::Failed(Failure::Timeout)
            },
            Event::Finished,
        ]
    );
    assert_eq!(Failure::RateLimited.to_string(), "rate_limited");
}
