//! Turning a judge's reply into an answer: reading its probabilities, then
//! settling them against the bar.
#![expect(
    clippy::unwrap_used,
    reason = "a broken fixture stops the benchmark before it measures"
)]

use std::hint::black_box;

use benchfmt::{CountingAlloc, Suite};
use chauffeur_judges::{Answer, AnsweredBy, Confidence, Milli, Outcomes, Probabilities, Question};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc::new();

fn main() {
    let yes_no = Question::noul(
        "Does this commit break Conventional Commits?",
        ["tool_call"],
    )
    .unwrap();
    let levels = Outcomes::new(["low", "medium", "high"]).unwrap();
    let scale = Question::new("How hard is the user's request?", levels, ["prompt"]).unwrap();
    let bar = Confidence::Bar(Milli::new(700).unwrap());
    let by = AnsweredBy {
        model: "jev".into(),
        placeholder: false,
    };
    let reply = [("yes", 0.94), ("no", 0.06)];
    let spread = [("low", 0.12), ("medium", 0.45), ("high", 0.43)];

    let settle = |question: &Question, reply: &[(&str, f64)]| {
        let p = Probabilities::read(question.outcomes(), reply.iter().copied()).unwrap();
        Answer::settle(question, p, &bar, by.clone())
    };

    Suite::from_env("chauffeur-judges")
        .with_alloc(&ALLOC)
        .bench("Read/outcomes=2", |b| {
            b.iter(|| Probabilities::read(yes_no.outcomes(), black_box(reply)));
        })
        .bench("Settle/outcomes=2", |b| {
            b.iter(|| settle(&yes_no, black_box(&reply)));
        })
        .bench("Settle/outcomes=3", |b| {
            b.iter(|| settle(&scale, black_box(&spread)));
        })
        .run();
}
