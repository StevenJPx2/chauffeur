//! Runs a rule for an event: redacts the evidence, asks the rule's judges on
//! a worker thread within a deadline, and keeps a bounded log of runs.

mod log;
mod redact;
mod run;

pub use log::Log;
pub use redact::{Masking, Prefix, REDACTED, RedactError, RedactionConfig, Redactor};
pub use run::{Context, Keys, Ran, run};
