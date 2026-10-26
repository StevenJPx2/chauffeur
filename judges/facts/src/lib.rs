//! The facts judge: plain code, no model. It answers a yes/no question from
//! the facts the question carries, by the rule configured for the question's
//! ID in `judge.json`. A question it has no rule for, or whose rule reads a
//! fact the question lacks, is left unanswered, so its capability applies
//! the same failure posture as for a model that did not answer.
//!
//! ```json
//! "facts": {
//!   "model-router/renew_context": {
//!     "all": [
//!       { "fact": "idle_seconds", "at_least": { "fact": "cache_seconds" } },
//!       { "fact": "context_tokens", "at_least": { "fact": "context_budget" } }
//!     ]
//!   }
//! }
//! ```

use std::collections::BTreeMap;

use chauffeur_core::{
    Answer, AnswerValue, Docket, Facts, Judge, Question, QuestionKind, Rulings, judge::unavailable,
};
use serde::Deserialize;

/// The name routes give this judge.
pub const NAME: &str = "facts";
/// Conditions one rule may hold.
const MAX_CONDITIONS: usize = 16;

/// Each question's rule, by namespaced question ID.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct FactsConfig(BTreeMap<String, FactRule>);

impl FactsConfig {
    /// The config, if every rule is well formed.
    ///
    /// # Errors
    ///
    /// Names the first rule out of bounds.
    pub fn checked(self) -> Result<Self, String> {
        for (id, rule) in &self.0 {
            rule.check()
                .map_err(|error| format!("facts.{id}: {error}"))?;
        }

        Ok(self)
    }

    /// Whether a rule answers question `id`.
    #[must_use]
    pub fn has(&self, id: &str) -> bool {
        self.0.contains_key(id)
    }
}

/// Yes when every condition holds (`all`), or when any one does (`any`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FactRule {
    All(Vec<Condition>),
    Any(Vec<Condition>),
}

impl FactRule {
    fn conditions(&self) -> &[Condition] {
        match self {
            Self::All(conditions) | Self::Any(conditions) => conditions,
        }
    }

    fn check(&self) -> Result<(), String> {
        let conditions = self.conditions();

        if conditions.is_empty() || conditions.len() > MAX_CONDITIONS {
            return Err(format!("a rule holds 1 to {MAX_CONDITIONS} conditions"));
        }

        conditions.iter().try_for_each(Condition::check)
    }

    /// Whether the rule holds for `facts`; `None` when a fact it reads is
    /// missing.
    fn holds(&self, facts: &Facts) -> Option<bool> {
        let mut results = self
            .conditions()
            .iter()
            .map(|condition| condition.holds(facts));

        match self {
            Self::All(_) => results.try_fold(true, |all, held| Some(all && held?)),
            Self::Any(_) => results.try_fold(false, |any, held| Some(any || held?)),
        }
    }
}

/// One comparison: `fact` at least, or at most, a value.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub fact: String,
    #[serde(default)]
    pub at_least: Option<Operand>,
    #[serde(default)]
    pub at_most: Option<Operand>,
}

impl Condition {
    fn check(&self) -> Result<(), String> {
        if self.fact.trim().is_empty() {
            return Err("a condition names its fact".into());
        }

        match (&self.at_least, &self.at_most) {
            (Some(operand), None) | (None, Some(operand)) => operand.check(),
            _ => Err(format!(
                "{}: a condition has exactly one of at_least and at_most",
                self.fact
            )),
        }
    }

    fn holds(&self, facts: &Facts) -> Option<bool> {
        #[expect(clippy::cast_precision_loss, reason = "facts are counts and seconds")]
        let value = *facts.get(&self.fact)? as f64;

        match (&self.at_least, &self.at_most) {
            (Some(bound), _) => Some(value >= bound.value(facts)?),
            (_, Some(bound)) => Some(value <= bound.value(facts)?),
            (None, None) => None,
        }
    }
}

/// A number, or another fact scaled by `times`.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum Operand {
    Number(f64),
    Fact {
        fact: String,
        #[serde(default = "once")]
        times: f64,
    },
}

const fn once() -> f64 {
    1.0
}

impl Operand {
    fn check(&self) -> Result<(), String> {
        match self {
            Self::Number(number) if number.is_finite() => Ok(()),
            Self::Fact { fact, times } if !fact.trim().is_empty() && times.is_finite() => Ok(()),
            _ => Err("a bound is a finite number or a fact with a finite `times`".into()),
        }
    }

    #[expect(clippy::cast_precision_loss, reason = "facts are counts and seconds")]
    fn value(&self, facts: &Facts) -> Option<f64> {
        match self {
            Self::Number(number) => Some(*number),
            Self::Fact { fact, times } => Some(*facts.get(fact)? as f64 * times),
        }
    }
}

/// The facts judge over its configured rules.
#[derive(Clone, Debug, Default)]
pub struct FactsJudge {
    rules: FactsConfig,
}

impl FactsJudge {
    #[must_use]
    pub fn new(rules: FactsConfig) -> Self {
        Self { rules }
    }

    /// A yes/no question's answer: certain either way. Other kinds, and a
    /// question without a rule or a fact its rule reads, get none.
    fn answer(&self, question: &Question) -> Option<Answer> {
        if question.kind != QuestionKind::Noul {
            return None;
        }

        let held = self.rules.0.get(&question.id)?.holds(&question.facts)?;

        Some(Answer {
            id: question.id.clone(),
            value: AnswerValue::Noul(if held { 1.0 } else { 0.0 }),
            confidence: Some(1.0),
        })
    }
}

impl Judge for FactsJudge {
    fn name(&self) -> &str {
        NAME
    }

    fn local(&self) -> bool {
        true
    }

    fn judge(&mut self, docket: &Docket<'_>) -> Rulings {
        Rulings {
            answers: Ok(docket
                .questions
                .iter()
                .filter_map(|question| self.answer(question))
                .collect()),
            reasoned: unavailable(docket.reasonings, "the facts judge does not reason"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn config() -> FactsConfig {
        serde_json::from_value::<FactsConfig>(serde_json::json!({
            "renew": {
                "all": [
                    { "fact": "idle_seconds", "at_least": { "fact": "cache_seconds" } },
                    { "fact": "context_tokens", "at_least": { "fact": "context_window", "times": 0.5 } }
                ]
            },
            "small": { "any": [{ "fact": "tokens", "at_most": 10 }] }
        }))
        .unwrap()
        .checked()
        .unwrap()
    }

    fn question(id: &str, facts: &[(&str, i64)]) -> Question {
        Question {
            id: id.into(),
            instructions: String::new(),
            kind: QuestionKind::Noul,
            facts: facts
                .iter()
                .map(|(name, value)| ((*name).to_string(), *value))
                .collect(),
        }
    }

    fn rule(judge: &mut FactsJudge, questions: &[Question]) -> Vec<Answer> {
        judge
            .judge(&Docket {
                state: "",
                questions,
                reasonings: &[],
                timeout: Duration::from_secs(1),
            })
            .answers
            .unwrap()
    }

    #[test]
    fn a_rule_answers_a_certain_yes_or_no_from_the_question_s_facts() {
        let mut judge = FactsJudge::new(config());
        let cold_and_long = question(
            "renew",
            &[
                ("idle_seconds", 900),
                ("cache_seconds", 300),
                ("context_tokens", 600_000),
                ("context_window", 1_000_000),
            ],
        );
        let warm = question(
            "renew",
            &[
                ("idle_seconds", 60),
                ("cache_seconds", 300),
                ("context_tokens", 600_000),
                ("context_window", 1_000_000),
            ],
        );

        let answers = rule(&mut judge, &[cold_and_long]);
        assert_eq!(answers[0].value, AnswerValue::Noul(1.0));
        assert_eq!(answers[0].confidence, Some(1.0));
        assert_eq!(rule(&mut judge, &[warm])[0].value, AnswerValue::Noul(0.0));
        assert_eq!(
            rule(&mut judge, &[question("small", &[("tokens", 4)])])[0].value,
            AnswerValue::Noul(1.0)
        );
        assert!(judge.local());
    }

    #[test]
    fn a_question_without_a_rule_or_a_fact_it_reads_goes_unanswered() {
        let mut judge = FactsJudge::new(config());

        assert!(rule(&mut judge, &[question("unknown", &[("tokens", 4)])]).is_empty());
        assert!(rule(&mut judge, &[question("renew", &[("idle_seconds", 900)])]).is_empty());
    }

    #[test]
    fn a_malformed_rule_is_refused() {
        for rule in [
            serde_json::json!({ "x": { "all": [] } }),
            serde_json::json!({ "x": { "all": [{ "fact": "a" }] } }),
            serde_json::json!({ "x": { "all": [{ "fact": "a", "at_least": 1, "at_most": 2 }] } }),
            serde_json::json!({ "x": { "all": [{ "fact": " ", "at_least": 1 }] } }),
        ] {
            let parsed = serde_json::from_value::<FactsConfig>(rule.clone())
                .map_err(|error| error.to_string())
                .and_then(FactsConfig::checked);

            assert!(parsed.is_err(), "{rule}");
        }
    }
}
