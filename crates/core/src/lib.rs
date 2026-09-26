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
pub mod judge;
pub mod protocol;

pub use contracts::{capability, effect, signal, system_one};
pub(crate) use features::learning;
pub use features::{backstop, redact};
pub use state::{situation, trace};

pub use backstop::{Backstop, BackstopConfig};
pub use capability::{Capability, PipeStep, Plan};
#[cfg(feature = "client")]
pub use client::{DEFAULT_DAEMON_URL, DaemonClient};
pub use config::{load_config, read_json_files};
pub use effect::{Delivery, Effect, PermissionDecision};
pub use engine::{Engine, Step};
pub use judge::{Judge, Pick, Rule};
pub use protocol::*;
pub use redact::{LearnedShapes, Prefix, RedactionConfig, Redactor, Shape, redact_secrets};
pub use signal::{
    AvailableModel, CatalogEntry, CodeModeNamespace, MAX_TEXT_BYTES, ModelRef, Resource, Signal,
    SignalKind,
};
pub use situation::Situation;
pub use system_one::{
    Answer, AnswerValue, ChoiceOption, Question, QuestionKind, SystemOne, SystemOneError,
    validate_answers,
};
pub use trace::Trace;
