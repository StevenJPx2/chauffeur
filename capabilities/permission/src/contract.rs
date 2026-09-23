//! Strict JSON skill contracts: the permission capability's configuration.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use chauffeur_core::{Answer, AnswerValue};
use serde::{Deserialize, Serialize};

pub const MAX_SKILL_BYTES: usize = 65_536;
pub const MAX_SKILLS: usize = 64;
const MAX_TEXT_BYTES: usize = 4_096;
const MAX_SKILL_DIRECTORY_ENTRIES: usize = 256;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    Allow,
    Deny,
    Ask,
    Prompt,
    Remind,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QuestionType {
    Choice,
    Score,
    Noul,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Branch {
    Positive,
    Negative,
    Uncertain,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillIdentity {
    pub id: String,
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MatchConditions {
    pub events: Vec<String>,
    pub actions: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Predicates {
    #[serde(default)]
    pub required_evidence: Vec<String>,
    #[serde(default)]
    pub forbidden_evidence: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub value: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DecisionQuestion {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: QuestionType,
    pub instructions: String,
    #[serde(default)]
    pub criteria: Vec<Criterion>,
    pub minimum_confidence: f32,
    #[serde(default)]
    pub positive_at_or_above: Option<f32>,
    #[serde(default)]
    pub negative_at_or_below: Option<f32>,
    #[serde(default)]
    pub choice_outcomes: Vec<ChoiceOutcome>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ChoiceOutcome {
    pub value: String,
    pub branch: Branch,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub effect: Effect,
    pub message: String,
    #[serde(default)]
    pub reminder: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillOutcomes {
    pub positive: Outcome,
    pub negative: Outcome,
    pub uncertain: Outcome,
    pub missing_evidence: Outcome,
    pub judge_failure: Outcome,
    pub cooldown: Outcome,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub schema_version: u32,
    pub identity: SkillIdentity,
    #[serde(rename = "match")]
    pub match_conditions: MatchConditions,
    pub predicates: Predicates,
    pub decision: DecisionQuestion,
    pub outcomes: SkillOutcomes,
    pub cooldown_seconds: u64,
}

/// Map a typed answer to the contract's branch. Confidence below the
/// contract's floor makes the branch uncertain.
pub fn resolve_branch(skill: &Skill, answer: &Answer) -> Result<Branch, String> {
    let question = &skill.decision;
    let branch = match (&question.kind, &answer.value) {
        (QuestionType::Choice, AnswerValue::Choice(choice)) => question
            .choice_outcomes
            .iter()
            .find(|mapping| mapping.value == *choice)
            .map(|mapping| mapping.branch)
            .ok_or_else(|| format!("unknown choice {choice}")),
        (QuestionType::Noul, AnswerValue::Noul(probability)) => {
            resolve_numeric(question, *probability, 1.0)
        }
        (QuestionType::Score, AnswerValue::Score(score)) => {
            resolve_numeric(question, *score, score_maximum(question)?)
        }
        _ => Err("answer type does not match the contract question".into()),
    }?;

    if answer.effective_confidence() < question.minimum_confidence {
        return Ok(Branch::Uncertain);
    }

    Ok(branch)
}

fn score_maximum(question: &DecisionQuestion) -> Result<f32, String> {
    let maximum = question.criteria.len().saturating_sub(1);
    let maximum = u8::try_from(maximum).map_err(|error| error.to_string())?;

    Ok(f32::from(maximum))
}

fn resolve_numeric(
    question: &DecisionQuestion,
    value: f32,
    maximum: f32,
) -> Result<Branch, String> {
    let positive = question
        .positive_at_or_above
        .ok_or("positive decision threshold is missing")?;
    let negative = question
        .negative_at_or_below
        .ok_or("negative decision threshold is missing")?;

    if !value.is_finite() || !(0.0..=maximum).contains(&value) {
        return Err(format!("decision value is outside [0, {maximum}]"));
    }

    Ok(if value >= positive {
        Branch::Positive
    } else if value <= negative {
        Branch::Negative
    } else {
        Branch::Uncertain
    })
}

/// Required evidence that is not `true`, then forbidden evidence that is.
pub fn missing_evidence(skill: &Skill, evidence: &HashMap<String, bool>) -> Vec<String> {
    let mut missing = skill
        .predicates
        .required_evidence
        .iter()
        .filter(|key| evidence.get(*key) != Some(&true))
        .cloned()
        .collect::<Vec<_>>();

    missing.extend(
        skill
            .predicates
            .forbidden_evidence
            .iter()
            .filter(|key| evidence.get(*key) == Some(&true))
            .cloned(),
    );

    missing
}

impl Skill {
    pub fn from_json(source: &[u8]) -> Result<Self, String> {
        if source.len() > MAX_SKILL_BYTES {
            return Err(format!("skill file exceeds {MAX_SKILL_BYTES} bytes"));
        }

        let skill: Self = serde_json::from_slice(source)
            .map_err(|error| format!("invalid skill JSON: {error}"))?;

        skill.validate()?;

        Ok(skill)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported schema_version {}",
                self.schema_version
            ));
        }
        validate_identifier(&self.identity.id, "skill id")?;
        validate_text(&self.identity.name, "skill name", 160)?;
        validate_version(&self.identity.version)?;
        validate_strings(&self.match_conditions.events, "match.events", 16, 120)?;
        validate_strings(&self.match_conditions.actions, "match.actions", 16, 120)?;
        if self.match_conditions.events.is_empty() || self.match_conditions.actions.is_empty() {
            return Err(
                "match.events and match.actions must each contain at least one value".into(),
            );
        }
        validate_strings(
            &self.predicates.required_evidence,
            "required_evidence",
            16,
            120,
        )?;
        validate_strings(
            &self.predicates.forbidden_evidence,
            "forbidden_evidence",
            16,
            120,
        )?;
        validate_predicate_sets(self)?;
        validate_question(&self.decision)?;
        validate_outcomes(&self.outcomes)?;

        if self.cooldown_seconds > 86_400 {
            return Err("cooldown_seconds exceeds 86400".into());
        }

        Ok(())
    }
}

pub fn load_skill(path: &Path) -> Result<Skill, String> {
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("stat {}: {error}", path.display()))?;

    if !metadata.is_file() {
        return Err(format!(
            "skill path {} is not a regular file",
            path.display()
        ));
    }

    let file = File::open(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let capacity =
        u64::try_from(MAX_SKILL_BYTES.saturating_add(1)).map_err(|error| error.to_string())?;
    let mut source = Vec::with_capacity(MAX_SKILL_BYTES);

    file.take(capacity)
        .read_to_end(&mut source)
        .map_err(|error| format!("read {}: {error}", path.display()))?;

    if source.len() > MAX_SKILL_BYTES {
        return Err(format!(
            "skill file {} exceeds {MAX_SKILL_BYTES} bytes",
            path.display()
        ));
    }

    Skill::from_json(&source).map_err(|error| format!("{}: {error}", path.display()))
}

pub fn load_skills(directory: &Path) -> Result<Vec<Skill>, String> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("read skills directory {}: {error}", directory.display()))?;
    let mut paths = Vec::new();
    let mut entry_count = 0;

    for entry in entries {
        if entry_count == MAX_SKILL_DIRECTORY_ENTRIES {
            return Err(format!(
                "skills directory contains more than {MAX_SKILL_DIRECTORY_ENTRIES} entries"
            ));
        }

        entry_count = entry_count.saturating_add(1);
        let entry = entry.map_err(|error| format!("read skills directory entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("inspect skills directory entry: {error}"))?;

        if file_type.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            if paths.len() == MAX_SKILLS {
                return Err(format!(
                    "skills directory contains more than {MAX_SKILLS} JSON files"
                ));
            }

            paths.push(entry.path());
        }
    }

    paths.sort();

    let mut skills = Vec::with_capacity(paths.len());
    let mut ids = HashSet::with_capacity(paths.len());

    for path in paths {
        let skill = load_skill(&path)?;

        if !ids.insert(skill.identity.id.clone()) {
            return Err(format!("duplicate skill id {}", skill.identity.id));
        }

        skills.push(skill);
    }

    Ok(skills)
}

fn validate_question(question: &DecisionQuestion) -> Result<(), String> {
    validate_identifier(&question.id, "decision id")?;
    validate_text(
        &question.instructions,
        "decision instructions",
        MAX_TEXT_BYTES,
    )?;
    validate_unit_interval(question.minimum_confidence, "minimum_confidence")?;

    if question.criteria.len() > 8 {
        return Err("decision.criteria has more than 8 entries".into());
    }

    let mut values = HashSet::new();

    for criterion in &question.criteria {
        validate_identifier(&criterion.value, "criterion value")?;
        validate_text(&criterion.description, "criterion description", 500)?;

        if !values.insert(criterion.value.as_str()) {
            return Err(format!("duplicate criterion value {}", criterion.value));
        }
    }

    match question.kind {
        QuestionType::Noul => validate_noul_question(question),
        QuestionType::Score => validate_score_question(question),
        QuestionType::Choice => validate_choice_question(question, &values),
    }
}

fn validate_noul_question(question: &DecisionQuestion) -> Result<(), String> {
    if !question.criteria.is_empty() || !question.choice_outcomes.is_empty() {
        return Err("noul questions cannot define criteria or choice_outcomes".into());
    }

    validate_numeric_thresholds(question, 1.0)
}

fn validate_score_question(question: &DecisionQuestion) -> Result<(), String> {
    if question.criteria.len() < 2 || !question.choice_outcomes.is_empty() {
        return Err("score questions require 2 to 8 criteria and no choice_outcomes".into());
    }

    validate_numeric_thresholds(question, score_maximum(question)?)
}

fn validate_choice_question(
    question: &DecisionQuestion,
    values: &HashSet<&str>,
) -> Result<(), String> {
    if question.criteria.len() < 2
        || question.positive_at_or_above.is_some()
        || question.negative_at_or_below.is_some()
    {
        return Err("choice questions require 2 to 8 criteria and use confidence only".into());
    }
    if question.choice_outcomes.len() != values.len() {
        return Err("choice_outcomes must map every criterion exactly once".into());
    }

    let mut mapped = HashSet::new();

    for outcome in &question.choice_outcomes {
        if !values.contains(outcome.value.as_str()) || !mapped.insert(outcome.value.as_str()) {
            return Err(format!(
                "invalid or duplicate choice outcome {}",
                outcome.value
            ));
        }
    }

    Ok(())
}

fn validate_numeric_thresholds(question: &DecisionQuestion, maximum: f32) -> Result<(), String> {
    let positive = question
        .positive_at_or_above
        .ok_or("positive_at_or_above is required")?;
    let negative = question
        .negative_at_or_below
        .ok_or("negative_at_or_below is required")?;

    if !positive.is_finite()
        || !negative.is_finite()
        || negative < 0.0
        || positive > maximum
        || negative >= positive
    {
        return Err(format!(
            "decision thresholds must be ordered within [0, {maximum}]"
        ));
    }

    Ok(())
}

fn validate_outcomes(outcomes: &SkillOutcomes) -> Result<(), String> {
    for (name, outcome) in [
        ("positive", &outcomes.positive),
        ("negative", &outcomes.negative),
        ("uncertain", &outcomes.uncertain),
        ("missing_evidence", &outcomes.missing_evidence),
        ("judge_failure", &outcomes.judge_failure),
        ("cooldown", &outcomes.cooldown),
    ] {
        validate_text(&outcome.message, &format!("outcomes.{name}.message"), 1_000)?;

        if let Some(reminder) = &outcome.reminder {
            validate_text(reminder, &format!("outcomes.{name}.reminder"), 1_000)?;
        }
    }

    for (name, outcome) in [
        ("uncertain", &outcomes.uncertain),
        ("missing_evidence", &outcomes.missing_evidence),
        ("judge_failure", &outcomes.judge_failure),
        ("cooldown", &outcomes.cooldown),
    ] {
        if outcome.effect == Effect::Allow {
            return Err(format!("outcomes.{name} cannot allow the action"));
        }
    }

    Ok(())
}

fn validate_predicate_sets(skill: &Skill) -> Result<(), String> {
    let required: HashSet<_> = skill.predicates.required_evidence.iter().collect();

    if skill
        .predicates
        .forbidden_evidence
        .iter()
        .any(|item| required.contains(item))
    {
        return Err("evidence cannot be both required and forbidden".into());
    }

    Ok(())
}

fn validate_strings(
    values: &[String],
    field: &str,
    limit: usize,
    item_limit: usize,
) -> Result<(), String> {
    if values.len() > limit {
        return Err(format!("{field} has more than {limit} entries"));
    }

    let mut seen = HashSet::new();

    for value in values {
        if value.is_empty() || value.len() > item_limit {
            return Err(format!(
                "{field} entries must contain 1 to {item_limit} bytes"
            ));
        }
        if !seen.insert(value) {
            return Err(format!("{field} contains duplicate value {value}"));
        }
    }

    Ok(())
}

fn validate_identifier(value: &str, field: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 120
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        return Err(format!(
            "{field} must use lowercase ASCII letters, digits, '.', '_' or '-'"
        ));
    }

    Ok(())
}

fn validate_version(value: &str) -> Result<(), String> {
    let segments: Vec<_> = value.split('.').collect();

    if segments.len() != 3
        || segments.iter().any(|part| {
            part.is_empty()
                || (part.len() > 1 && part.starts_with('0'))
                || part.parse::<u32>().is_err()
        })
    {
        return Err("identity.version must be a numeric major.minor.patch version".into());
    }

    Ok(())
}

fn validate_text(value: &str, field: &str, max_bytes: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max_bytes {
        return Err(format!("{field} must contain 1 to {max_bytes} bytes"));
    }

    Ok(())
}

fn validate_unit_interval(value: f32, field: &str) -> Result<(), String> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(format!("{field} must be within [0, 1]"))
    }
}
