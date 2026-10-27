//! Questions, the answers judges give, and the confidence that settles an
//! answer into one outcome or `unsure`; `models.json`, each tier's models in
//! fallback order, and the formats that ask them.
//!
//! ```
//! use chauffeur_judges::{Answer, AnsweredBy, Confidence, Milli, Probabilities, Question, Settled};
//!
//! let question = Question::noul("Does this commit message break Conventional Commits?", ["commit"])?;
//! let p = Probabilities::read(question.outcomes(), [("yes", 0.94), ("no", 0.06)])?;
//! let bar = Confidence::Bar(Milli::new(700).ok_or("bar")?);
//! let by = AnsweredBy { model: "jev".into(), placeholder: false };
//!
//! let answer = Answer::settle(&question, p, &bar, by);
//! assert_eq!(answer.outcome(), &Settled::Outcome("yes".into()));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod confidence;
mod config;
mod format;
mod placeholder;
#[cfg(kani)]
mod proofs;
mod question;
#[cfg(feature = "systemone")]
mod systemone;
mod tier;

pub use confidence::{
    Answer, AnsweredBy, Confidence, ConfidenceError, Milli, Probabilities, Settled,
};
pub use config::{
    ConfigError, KeySource, ModelEntry, Models, PlaceholderEntry, SystemOneEntry, Tier,
};
pub use format::{Evidence, Format};
pub use placeholder::Placeholder;
pub use question::{Label, Outcomes, Question, QuestionError};
#[cfg(feature = "systemone")]
pub use systemone::SystemOne;
pub use tier::{Fallback, Model, NamedJudge};
