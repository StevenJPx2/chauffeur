use std::collections::BTreeMap;

use chauffeur_judges::{Confidence, Milli, Tier};

use super::error::RuleError;
use super::file::{ConfidenceFile, JudgeFile, StepFile};

/// A named judge in a rule: its tier, an optional model, and its confidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JudgeSpec {
    /// The tier whose models answer.
    pub tier: Tier,
    /// The model asked first, before the tier's fallback order.
    pub model: Option<String>,
    /// How likely an answer must be to settle.
    pub confidence: Confidence,
}

/// The judge `name` as written in `judges`, its confidence checked to be
/// probabilities.
pub(super) fn judge_spec(rule: &str, name: &str, judge: JudgeFile) -> Result<JudgeSpec, RuleError> {
    let invalid = |value: f64| RuleError::Confidence {
        rule: rule.to_owned(),
        step: name.to_owned(),
        reason: format!("confidence {value} is not a probability from 0 to 1"),
    };
    let confidence = match judge.confidence {
        None => Confidence::Likeliest,
        Some(ConfidenceFile::Bar(bar)) => {
            Confidence::Bar(Milli::from_f64(bar).ok_or_else(|| invalid(bar))?)
        }
        Some(ConfidenceFile::PerOutcome(bars)) => Confidence::PerOutcome(
            bars.into_iter()
                .map(|(outcome, bar)| {
                    Milli::from_f64(bar)
                        .map(|bar| (outcome, bar))
                        .ok_or_else(|| invalid(bar))
                })
                .collect::<Result<_, _>>()?,
        ),
    };
    Ok(JudgeSpec {
        tier: judge.tier,
        model: judge.model,
        confidence,
    })
}

/// The step's judge from `judges`, or a tier named directly, which settles on
/// the likeliest outcome.
pub(super) fn resolve_judge<'a>(
    rule: &str,
    step: &StepFile,
    judges: &'a mut BTreeMap<String, JudgeSpec>,
) -> Result<&'a JudgeSpec, RuleError> {
    if !judges.contains_key(&step.judge) {
        let tier = Tier::from_name(&step.judge).ok_or_else(|| RuleError::UnknownJudge {
            rule: rule.to_owned(),
            step: step.id.clone(),
            judge: step.judge.clone(),
        })?;
        judges.insert(
            step.judge.clone(),
            JudgeSpec {
                tier,
                model: None,
                confidence: Confidence::Likeliest,
            },
        );
    }
    judges
        .get(&step.judge)
        .ok_or_else(|| RuleError::UnknownJudge {
            rule: rule.to_owned(),
            step: step.id.clone(),
            judge: step.judge.clone(),
        })
}
