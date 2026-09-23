//! Chauffeur SDK: domain model, plugin registry, steering runtime, durable
//! reminder queue, wire protocol, dispatch, and daemon client.

pub mod client;
pub mod context;
pub mod dispatch;
pub mod plugin;
pub mod protocol;
pub mod queue;
pub mod rule;
pub mod runtime;
pub mod skill;
pub mod steer;
pub mod target;

pub use client::{DEFAULT_DAEMON_URL, DaemonClient};
pub use context::{AgentContext, Brief, Notice, ToolCall};
pub use dispatch::{dispatch, dispatch_method};
pub use plugin::{Plugin, compose};
pub use protocol::*;
pub use queue::{InMemoryReminderQueue, JsonReminderQueue, QueuedReminder, ReminderQueue};
pub use rule::{Gate, Rule, Threshold, load_rules};
pub use runtime::Runtime;
pub use skill::{
    Branch, ChoiceOutcome, Criterion, DecisionAnswer, DecisionQuestion, DecisionValue, Effect,
    EvaluationStatus, MatchConditions, Outcome, Predicates, QuestionType, Skill, SkillContext,
    SkillIdentity, SkillOutcomes, SkillResult, load_skill, load_skills,
};
pub use steer::{Error, Judge, Reminder, Steerer, Urgency, Verdict};
pub use target::Target;
