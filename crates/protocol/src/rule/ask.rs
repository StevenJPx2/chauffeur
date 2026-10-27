use std::collections::BTreeMap;

use chauffeur_judges::{Label, Outcomes, Question};

use super::error::RuleError;
use super::file::{AskFile, AskRef, LabelFile, StepFile};

/// The step's question: its inline ask, or the one it names in `asks`.
pub(super) fn resolve_ask(
    rule: &str,
    step: &StepFile,
    asks: &BTreeMap<String, AskFile>,
) -> Result<Question, RuleError> {
    let ask = match &step.ask {
        AskRef::Inline(ask) => ask,
        AskRef::Named(name) => asks.get(name).ok_or_else(|| RuleError::UnknownAsk {
            rule: rule.to_owned(),
            step: step.id.clone(),
            ask: name.clone(),
        })?,
    };
    let labels = ask.outcomes.iter().map(|label| match label {
        LabelFile::Value(value) => Label::new(value.clone()),
        LabelFile::Described { value, description } => {
            Label::described(value.clone(), description.clone())
        }
    });
    Outcomes::new(labels)
        .and_then(|outcomes| {
            Question::new(
                ask.instructions.clone(),
                outcomes,
                ask.evidence.iter().cloned(),
            )
        })
        .map_err(|source| RuleError::Question {
            rule: rule.to_owned(),
            step: step.id.clone(),
            source,
        })
}
