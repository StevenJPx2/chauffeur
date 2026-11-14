use std::collections::BTreeMap;

use chauffeur_core::{Plan, Step};
use chauffeur_judges::{ConfidenceError, Question};

use super::ask;
use super::error::RuleError;
use super::file::{AskFile, JudgeFile, StepFile};
use super::judge::{self, JudgeSpec};
use crate::limits::{Limit, count};

/// A strategy: the judges its steps name, and its steps as a core plan.
/// A rule holds one, and a request may send one inline.
#[derive(Debug)]
pub struct Strategy {
    judges: BTreeMap<String, JudgeSpec>,
    plan: Plan<Question>,
}

impl AsRef<Self> for Strategy {
    fn as_ref(&self) -> &Self {
        self
    }
}

impl Strategy {
    /// Checks the strategy of `id`: each step's judge and question, and each
    /// judge's confidence against the outcomes of the steps it answers.
    pub(crate) fn build(
        id: &str,
        judges: BTreeMap<String, JudgeFile>,
        asks: &BTreeMap<String, AskFile>,
        strategy: Vec<StepFile>,
    ) -> Result<Self, RuleError> {
        let limit = |source| RuleError::Limit {
            rule: id.to_owned(),
            source,
        };
        Limit::Steps.check(count(strategy.len())).map_err(limit)?;
        // Every ask, used or not, so an unused one cannot carry unbounded outcomes.
        for ask in asks.values() {
            Limit::Outcomes
                .check(count(ask.outcomes.len()))
                .map_err(limit)?;
        }
        if strategy.iter().any(|step| step.id.trim().is_empty()) {
            return Err(RuleError::BlankStep {
                rule: id.to_owned(),
            });
        }
        let mut specs = BTreeMap::new();
        for (name, judge) in judges {
            specs.insert(name.clone(), judge::judge_spec(id, &name, judge)?);
        }
        let mut steps = Vec::with_capacity(strategy.len());
        for step in strategy {
            let question = ask::resolve_ask(id, &step, asks)?;
            let spec = judge::resolve_judge(id, &step, &mut specs)?;
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
        Ok(Self {
            judges: specs,
            plan,
        })
    }

    /// The judges its steps name, including tiers named directly.
    #[must_use]
    pub const fn judges(&self) -> &BTreeMap<String, JudgeSpec> {
        &self.judges
    }

    /// Its steps as a core plan.
    #[must_use]
    pub const fn plan(&self) -> &Plan<Question> {
        &self.plan
    }
}
