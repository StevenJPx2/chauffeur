//! Runs a rule for an event: redacts the evidence and asks the rule's judges
//! on a worker thread within a deadline.

mod redact;
mod run;

pub use redact::{Masking, Prefix, REDACTED, RedactError, RedactionConfig, Redactor};
pub use run::{Context, Keys, Ran, run};
