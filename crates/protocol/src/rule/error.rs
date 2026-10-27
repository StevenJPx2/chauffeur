use chauffeur_core::PlanError;
use chauffeur_judges::QuestionError;

/// Why a rule file was refused when it loaded. Each names the rule.
#[derive(Debug, thiserror::Error)]
pub enum RuleError {
    /// The file is not a valid rule.
    #[error("rule `{rule}` is not valid: {source}")]
    Parse {
        /// The rule's ID.
        rule: String,
        /// Why.
        source: serde_json::Error,
    },
    /// A step names a judge that is neither in `judges` nor a tier.
    #[error(
        "rule `{rule}`: step `{step}` names judge `{judge}`, which is neither in `judges` nor a tier"
    )]
    UnknownJudge {
        /// The rule's ID.
        rule: String,
        /// The step.
        step: String,
        /// The judge it names.
        judge: String,
    },
    /// A step names an ask that is not in `asks`.
    #[error("rule `{rule}`: step `{step}` names ask `{ask}`, which is not in `asks`")]
    UnknownAsk {
        /// The rule's ID.
        rule: String,
        /// The step.
        step: String,
        /// The ask it names.
        ask: String,
    },
    /// A step's question is not valid.
    #[error("rule `{rule}`: step `{step}`: {source}")]
    Question {
        /// The rule's ID.
        rule: String,
        /// The step.
        step: String,
        /// Why.
        source: QuestionError,
    },
    /// A judge's confidence is not a probability, or does not fit a step's outcomes.
    #[error("rule `{rule}`: step `{step}`: {reason}")]
    Confidence {
        /// The rule's ID.
        rule: String,
        /// The step.
        step: String,
        /// Why.
        reason: String,
    },
    /// The strategy is not a valid plan.
    #[error("rule `{rule}`: {source}")]
    Plan {
        /// The rule's ID.
        rule: String,
        /// Why.
        source: PlanError,
    },
    /// A `then` entry is not valid.
    #[error("rule `{rule}`: then entry {index} {reason}")]
    Then {
        /// The rule's ID.
        rule: String,
        /// The entry's position, from 0.
        index: usize,
        /// Why.
        reason: String,
    },
}
