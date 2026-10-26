//! Chauffeur core: generic, bounded judgment plans and the state machine that
//! runs them.
//!
//! A [`Plan`] is an ordered graph of named steps. Each step names the
//! [`Judge`] that answers it and carries a request of the caller's type `Q`;
//! answers are [`Outcome`]s over the caller's type `O`. [`Gate`]s on a step
//! decide from earlier steps' outcomes whether it runs or is skipped. Core
//! knows nothing about what requests or outcomes mean.
//!
//! A judge is anything that answers a request: a fixed rule, a System One
//! classifier, a System Two reasoner or a person. [`Plan::run`] runs a plan to
//! its end against a [`Panel`] of named judges.
//!
//! To drive a plan by hand instead, [`Plan::start`] returns one [`Transition`]
//! per step forward: the [`Next`] thing to do — answer a [`Round`] or read the
//! [`Verdict`] — and the [`Event`]s that the step produced. Core calls no user
//! code other than judges; observers read the events.
//!
//! # Examples
//!
//! A deterministic rule screens; a model reviews only what the rule cannot
//! decide.
//!
//! ```
//! use chauffeur_core::{Outcome, Panel, Resolution, ask};
//!
//! #[derive(Debug, Clone, PartialEq)]
//! enum Answer { Yes, No, Unsure }
//!
//! let rule = |evidence: &&str| match *evidence {
//!     e if e.contains("TODO") => Outcome::Value(Answer::Yes),
//!     _ => Outcome::Value(Answer::Unsure),
//! };
//! let model = |_: &&str| Outcome::Value(Answer::No);
//! let panel = Panel::new().with("rule", &rule).with("model", &model);
//!
//! let plan = |evidence| {
//!     ask("screen", "rule", evidence)
//!         .then_if_any(Outcome::Value(Answer::Unsure), ask("review", "model", evidence))
//! };
//!
//! let run = plan("fix the TODO in parse()").run(&panel)?;
//! assert_eq!(run.verdict.get("review"), Some(&Resolution::Skipped));
//!
//! let run = plan("the tests pass").run(&panel)?;
//! assert_eq!(
//!     run.verdict.get("review"),
//!     Some(&Resolution::Settled(Outcome::Value(Answer::No))),
//! );
//! # Ok::<(), chauffeur_core::PlanError>(())
//! ```

mod event;
mod gate;
mod judge;
mod plan;
#[cfg(kani)]
mod proofs;
mod state;

pub use event::{Event, Resolution, Verdict};
pub use judge::{Judge, Panel};
pub use plan::{Gate, Mode, Outcome, Plan, Step, When, all, ask};
pub use state::{AnswerError, Finished, Next, PlanError, Question, Rejected, Round, Transition};
