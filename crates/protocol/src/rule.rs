use std::collections::BTreeMap;
use std::time::Duration;

use chauffeur_core::{Plan, PlanError, Step};
use chauffeur_judges::{
    Confidence, ConfidenceError, Label, Milli, Outcomes, Question, QuestionError, Tier,
};
use serde::Deserialize;
use serde_json::Value;

use crate::event::Event;
use crate::then::{Expect, Then};
use crate::when::When;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleFile {
    on: String,
    #[serde(default)]
    when: When,
    #[serde(default)]
    judges: BTreeMap<String, JudgeFile>,
    #[serde(default)]
    asks: BTreeMap<String, AskFile>,
    strategy: Vec<StepFile>,
    #[serde(default)]
    then: Vec<ThenFile>,
    cooldown_seconds: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JudgeFile {
    tier: Tier,
    model: Option<String>,
    confidence: Option<ConfidenceFile>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ConfidenceFile {
    Bar(f64),
    PerOutcome(BTreeMap<String, f64>),
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AskFile {
    instructions: String,
    outcomes: Vec<LabelFile>,
    #[serde(default)]
    evidence: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
enum LabelFile {
    Value(String),
    Described { value: String, description: String },
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AskRef {
    Named(String),
    Inline(AskFile),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepFile {
    id: String,
    judge: String,
    ask: AskRef,
}

#[derive(Deserialize)]
struct ThenFile {
    #[serde(rename = "if", default)]
    needs: BTreeMap<String, String>,
    #[serde(flatten)]
    does: BTreeMap<String, Value>,
}

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
            judges.insert(name.clone(), judge_spec(id, &name, judge)?);
        }
        let mut steps = Vec::with_capacity(file.strategy.len());
        for step in file.strategy {
            let question = resolve_ask(id, &step, &file.asks)?;
            let spec = resolve_judge(id, &step, &mut judges)?;
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
            .map(|(index, entry)| resolve_then(id, index, entry, &plan))
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

fn judge_spec(rule: &str, name: &str, judge: JudgeFile) -> Result<JudgeSpec, RuleError> {
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
fn resolve_judge<'a>(
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

fn resolve_ask(
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

fn resolve_then(
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
