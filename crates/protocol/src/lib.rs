//! Rule files, the events they hook, and the actions or intents they return;
//! `navigate` requests, their limits, and what a navigation reports.
//!
//! ```
//! use chauffeur_protocol::{Event, Rule};
//!
//! let rule = Rule::parse("conventional-commits", r#"{
//!   "on": "tool.after",
//!   "when": { "tools": ["shell"], "input": { "contains": "git commit" } },
//!   "judges": { "quick": { "tier": "system-one", "confidence": 0.7 } },
//!   "strategy": [{ "id": "nonconventional", "judge": "quick",
//!                  "ask": { "outcomes": ["yes", "no"], "evidence": ["tool_call"],
//!                           "instructions": "Does this commit message break Conventional Commits?" } }],
//!   "then": [{ "if": { "nonconventional": "yes" }, "message.append": { "parts": [{ "text": "Use Conventional Commits." }], "delivery": "now" } }]
//! }"#)?;
//!
//! let commit = Event::parse(r#"{ "event": "tool.after", "session": "s1", "tool": "shell",
//!                                 "input": { "command": "git commit -m 'fixed stuff'" } }"#)?;
//! let listing = Event::parse(r#"{ "event": "tool.after", "session": "s1", "tool": "shell",
//!                                  "input": { "command": "ls" } }"#)?;
//! assert!(rule.hooks(&commit));
//! assert!(!rule.hooks(&listing));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod event;
mod intent;
mod limits;
pub mod progress;
mod request;
mod rule;
mod then;
mod when;

pub use event::{Event, EventError};
pub use intent::Intent;
pub use limits::{Exceeded, Limit};
pub use request::{DEFAULT_DEADLINE, Navigate, PROTOCOLS, Rejection, Target};
pub use rule::{JudgeSpec, Rule, RuleError, Strategy};
pub use then::{Expect, Then};
pub use when::When;
