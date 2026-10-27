//! Chauffeur's core: a plan of named steps, each asked of a named judge, and
//! the events and verdict of running it. It has no dependencies and knows
//! nothing of models, transports or harnesses.
//!
//! ```
//! use chauffeur_core::{Outcome, Panel, Plan, Step};
//!
//! let plan = Plan::new([
//!     Step::new("commit", "system-one", "Is this message conventional?"),
//!     Step::new("rule", "facts", "Does the diff touch migrations?"),
//! ])?;
//! let yes = |_: &&str| Outcome::Value("yes");
//! let no = |_: &&str| Outcome::Value("no");
//! let panel = Panel::new().with("system-one", &yes).with("facts", &no);
//!
//! let run = plan.run(&panel)?;
//! assert_eq!(run.verdict.get("commit"), Some(&Outcome::Value("yes")));
//! assert_eq!(run.verdict.get("rule"), Some(&Outcome::Value("no")));
//! # Ok::<(), chauffeur_core::PlanError>(())
//! ```

mod event;
mod judge;
mod plan;

pub use event::{Event, Run, Verdict};
pub use judge::{Judge, Panel};
pub use plan::{Failure, Outcome, Plan, PlanError, Step};
