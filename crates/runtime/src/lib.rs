//! Runs a rule for an event: redacts the evidence and asks the rule's judges
//! on a pooled worker thread within a deadline.

mod redact;
mod run;

pub use redact::{Masking, Prefix, REDACTED, RedactError, RedactionConfig, Redactor};
pub use run::{Context, Job, Keys, Pool, Ran, run};
