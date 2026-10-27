use chauffeur_core::Plan;
use chauffeur_judges::Question;

use super::error::RuleError;
use super::file::ThenFile;
use crate::then::{Expect, Then};

/// The `then` entry at `index`, its `if` checked against the plan's steps
/// and their outcomes, with exactly one action or intent.
pub(super) fn resolve_then(
    rule: &str,
    index: usize,
    entry: ThenFile,
    plan: &Plan<Question>,
) -> Result<Then, RuleError> {
    let refuse = |reason: String| RuleError::Then {
        rule: rule.to_owned(),
        index,
        reason,
    };
    let mut needs = Vec::with_capacity(entry.needs.len());
    for (step_id, wanted) in entry.needs {
        let step = plan
            .steps()
            .iter()
            .find(|step| step.id() == step_id)
            .ok_or_else(|| refuse(format!("names step `{step_id}`, which the strategy lacks")))?;
        let expect = Expect::parse(&wanted);
        if let Expect::Outcome(outcome) = &expect
            && step.request().outcomes().position(outcome).is_none()
        {
            return Err(refuse(format!(
                "expects `{outcome}` from step `{step_id}`, which is not one of its outcomes"
            )));
        }
        needs.push((step_id, expect));
    }
    let mut does = entry.does.into_iter();
    match (does.next(), does.next()) {
        (Some((name, payload)), None) => Ok(Then::new(needs, name, payload)),
        _ => Err(refuse(
            "needs exactly one action or intent beside `if`".to_owned(),
        )),
    }
}
