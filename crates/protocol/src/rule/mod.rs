mod ask;
mod error;
mod file;
mod judge;
mod then;

use std::collections::BTreeMap;
use std::time::Duration;

use chauffeur_core::{Plan, Step};
use chauffeur_judges::{ConfidenceError, Question};
pub use error::RuleError;
pub use judge::JudgeSpec;

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
    judges: BTreeMap<String, JudgeSpec>,
    plan: Plan<Question>,
    then: Vec<Then>,
    cooldown: Option<Duration>,
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
        let mut judges = BTreeMap::new();
        for (name, judge) in file.judges {
            judges.insert(name.clone(), judge::judge_spec(id, &name, judge)?);
        }
        let mut steps = Vec::with_capacity(file.strategy.len());
        for step in file.strategy {
            let question = ask::resolve_ask(id, &step, &file.asks)?;
            let spec = judge::resolve_judge(id, &step, &mut judges)?;
            spec.confidence
                .check(question.outcomes())
                .map_err(|error: ConfidenceError| RuleError::Confidence {
                    rule: id.to_owned(),
                    step: step.id.clone(),
                    reason: error.to_string(),
                })?;
            steps.push(Step::new(step.id, step.judge, question));
        }
        let plan = Plan::new(steps).map_err(|source| RuleError::Plan {
            rule: id.to_owned(),
            source,
        })?;
        let then = file
            .then
            .into_iter()
            .enumerate()
            .map(|(index, entry)| then::resolve_then(id, index, entry, &plan))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            id: id.to_owned(),
            on: file.on,
            when: file.when,
            judges,
            plan,
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

    /// The judges its steps name, including tiers named directly.
    #[must_use]
    pub const fn judges(&self) -> &BTreeMap<String, JudgeSpec> {
        &self.judges
    }

    /// Its strategy as a core plan.
    #[must_use]
    pub const fn plan(&self) -> &Plan<Question> {
        &self.plan
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
