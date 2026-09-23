//! Chauffeur SDK: the Sense → Classify → Act engine, System One interface,
//! capability, provider, and rule-plugin contracts, redaction, and the
//! daemon client.

pub mod backstop;
pub mod capability;
pub mod client;
pub mod config;
pub mod effect;
pub mod engine;
pub mod plugin;
pub mod protocol;
pub mod provider;
pub mod redact;
pub mod rule;
pub mod signal;
pub mod situation;
pub mod system_one;
pub mod trace;

pub use backstop::{Backstop, BackstopConfig};
pub use capability::{Capability, Plan};
pub use client::{DEFAULT_DAEMON_URL, DaemonClient};
pub use config::{load_config, read_json_files};
pub use effect::{Effect, PermissionDecision};
pub use engine::Engine;
pub use plugin::{Plugin, compose};
pub use protocol::*;
pub use provider::{Provider, Tier};
pub use redact::redact_secrets;
pub use rule::{Gate, IdleFacts, Rule, Threshold, load_rules};
pub use signal::{
    AvailableModel, CatalogEntry, MAX_TEXT_BYTES, ModelRef, Resource, Signal, SignalKind,
};
pub use situation::Situation;
pub use system_one::{
    Answer, AnswerValue, ChoiceOption, Question, QuestionKind, SystemOne, SystemOneError,
    validate_answers,
};
pub use trace::Trace;
