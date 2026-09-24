//! Chauffeur's core: the Sense → Classify → Act engine, the System One
//! interface, the capability contract, the host vocabulary (signals and
//! effects), the irreversible-harm backstop, redaction, and the daemon
//! client. It knows no capability; capabilities own their own concepts.

pub mod backstop;
pub mod capability;
#[cfg(feature = "client")]
pub mod client;
pub mod config;
pub mod effect;
pub mod engine;
pub mod protocol;
pub mod redact;
pub mod signal;
pub mod situation;
pub mod system_one;
pub mod trace;

pub use backstop::{Backstop, BackstopConfig};
pub use capability::{Capability, Plan};
#[cfg(feature = "client")]
pub use client::{DEFAULT_DAEMON_URL, DaemonClient};
pub use config::{load_config, read_json_files};
pub use effect::{Delivery, Effect, PermissionDecision};
pub use engine::{Engine, Step};
pub use protocol::*;
pub use redact::redact_secrets;
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
