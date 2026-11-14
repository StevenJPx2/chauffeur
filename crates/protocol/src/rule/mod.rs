mod append;
mod ask;
mod error;
mod file;
mod judge;
mod strategy;
mod then;

use std::collections::BTreeMap;
use std::time::Duration;

use chauffeur_core::Plan;
use chauffeur_judges::Question;
pub use error::RuleError;
pub use file::{AskFile, JudgeFile, StepFile};
pub use judge::JudgeSpec;
pub use strategy::Strategy;

use crate::event::Event;
use crate::then::Then;
use crate::when::When;
use file::RuleFile;

/// A loaded rule: when it runs, the strategy it runs, and what it returns.
#[derive(Debug)]
pub struct Rule {
    id: String,
    on: String,
    when: When,
    strategy: Strategy,
    then: Vec<Then>,
    cooldown: Option<Duration>,
}

impl AsRef<Strategy> for Rule {
    fn as_ref(&self) -> &Strategy {
        &self.strategy
    }
}

impl Rule {
    /// Parses and checks the rule file `text`, whose ID is `id`.
    ///
    /// # Errors
    ///
    /// A [`RuleError`] naming the rule and its first problem.
    pub fn parse(id: &str, text: &str) -> Result<Self, RuleError> {
        let file: RuleFile = serde_json::from_str(text).map_err(|source| RuleError::Parse {
            rule: id.to_owned(),
            source,
        })?;
        let strategy = Strategy::build(id, file.judges, &file.asks, file.strategy)?;
        let then = file
            .then
            .into_iter()
            .enumerate()
            .map(|(index, entry)| then::resolve_then(id, index, entry, strategy.plan()))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            id: id.to_owned(),
            on: file.on,
            when: file.when,
            strategy,
            then,
            cooldown: file.cooldown_seconds.map(Duration::from_secs),
        })
    }

    /// Whether the rule runs for `event`: its `on` names it and its `when` matches.
    #[must_use]
    pub fn hooks(&self, event: &Event) -> bool {
        self.on == event.name() && self.when.matches(event)
    }

    /// The rule's ID, its path under its folder.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The strategy it runs.
    #[must_use]
    pub const fn strategy(&self) -> &Strategy {
        &self.strategy
    }

    /// The judges its steps name, including tiers named directly.
    #[must_use]
    pub const fn judges(&self) -> &BTreeMap<String, JudgeSpec> {
        self.strategy.judges()
    }

    /// Its strategy as a core plan.
    #[must_use]
    pub const fn plan(&self) -> &Plan<Question> {
        self.strategy.plan()
    }

    /// Its `then` entries, in order.
    #[must_use]
    pub fn then(&self) -> &[Then] {
        &self.then
    }

    /// How long after it returns something it stays quiet in the same session.
    #[must_use]
    pub const fn cooldown(&self) -> Option<Duration> {
        self.cooldown
    }
}
