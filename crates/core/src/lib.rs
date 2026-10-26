//! Chauffeur's core: the Sense → Classify → Act engine, the System One
//! interface, the capability contract, the host vocabulary (signals and
//! effects), the irreversible-harm backstop, redaction, and the daemon
//! client. It knows no capability; capabilities own their own concepts.

mod contracts;
mod features;
mod state;

#[cfg(feature = "client")]
pub mod client;
pub mod config;
pub mod engine;
pub mod judgment;
pub mod protocol;
pub mod template;
pub mod watch;

pub use contracts::{capability, effect, judge, signal, system_one, system_two};
pub(crate) use features::learning;
pub use features::learning::{LearningConfig, LearningTexts};
pub use features::{backstop, redact};
pub use state::{situation, trace};

pub use backstop::{Backstop, BackstopConfig, BackstopTexts};
pub use capability::{Capability, Inference, PipeStep, Plan, Round, RunId};
#[cfg(feature = "client")]
pub use client::{DEFAULT_DAEMON_URL, DaemonClient};
pub use config::{load_config, load_layered, read_json_files};
pub use effect::{Delivery, Effect, PermissionDecision, SkillSafety};
pub use engine::{Engine, Step};
pub use judge::{
    Docket, Facts, Judge, Routes, Rulings, SYSTEM_ONE, SYSTEM_TWO, SystemOneJudge, SystemTwoJudge,
};
pub use judgment::{Confidence, Judged, Judging, Judgment, Pick, Rule, Threshold};
pub use protocol::*;
pub use redact::{LearnedShapes, Prefix, RedactionConfig, Redactor, Shape, redact_secrets};
pub use signal::{
    AvailableModel, CatalogEntry, CodeModeNamespace, MAX_PROMPT_BYTES, MAX_RESOURCE_BYTES,
    MAX_RESOURCES_TOTAL_BYTES, MAX_TEXT_BYTES, MAX_TODOS, ModelRef, Resource, RulebookCommand,
    Signal, SignalKind, SkillFile, Todo, TodoState, TodoStatus,
};
pub use situation::{Situation, SituationTexts};
pub use system_one::{
    Answer, AnswerValue, ChoiceOption, Question, QuestionKind, SystemOne, SystemOneError,
    validate_answers,
};
pub use system_two::{
    AttemptMetadata, Backend, Completion, CompletionMetadata, Evidence, Reasoned, Reasoning,
    ReasoningAttempt, ReasoningFailure, Schema, SystemTwo, Usage, validate_verdict,
};
pub use template::Template;
pub use trace::Trace;
pub use watch::{Reloading, Watch};
