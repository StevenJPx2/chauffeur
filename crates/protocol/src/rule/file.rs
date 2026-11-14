//! A rule file as written, before its checks.

use std::collections::BTreeMap;

use chauffeur_judges::Tier;
use serde::Deserialize;
use serde_json::Value;

use crate::when::When;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleFile {
    pub on: String,
    #[serde(default)]
    pub when: When,
    #[serde(default)]
    pub judges: BTreeMap<String, JudgeFile>,
    #[serde(default)]
    pub asks: BTreeMap<String, AskFile>,
    pub strategy: Vec<StepFile>,
    #[serde(default)]
    pub then: Vec<ThenFile>,
    pub cooldown_seconds: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgeFile {
    pub tier: Tier,
    pub model: Option<String>,
    pub confidence: Option<ConfidenceFile>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum ConfidenceFile {
    Bar(f64),
    PerOutcome(BTreeMap<String, f64>),
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskFile {
    pub instructions: String,
    pub outcomes: Vec<LabelFile>,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub enum LabelFile {
    Value(String),
    Described { value: String, description: String },
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum AskRef {
    Named(String),
    Inline(AskFile),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StepFile {
    pub id: String,
    pub judge: String,
    pub ask: AskRef,
}

#[derive(Deserialize)]
pub struct ThenFile {
    #[serde(rename = "if", default)]
    pub needs: BTreeMap<String, String>,
    #[serde(flatten)]
    pub does: BTreeMap<String, Value>,
}
