//! A rule file as written, before its checks.

use std::collections::BTreeMap;

use chauffeur_judges::Tier;
use serde::Deserialize;
use serde_json::Value;

use crate::when::When;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RuleFile {
    pub(super) on: String,
    #[serde(default)]
    pub(super) when: When,
    #[serde(default)]
    pub(super) judges: BTreeMap<String, JudgeFile>,
    #[serde(default)]
    pub(super) asks: BTreeMap<String, AskFile>,
    pub(super) strategy: Vec<StepFile>,
    #[serde(default)]
    pub(super) then: Vec<ThenFile>,
    pub(super) cooldown_seconds: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JudgeFile {
    pub(super) tier: Tier,
    pub(super) model: Option<String>,
    pub(super) confidence: Option<ConfidenceFile>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum ConfidenceFile {
    Bar(f64),
    PerOutcome(BTreeMap<String, f64>),
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AskFile {
    pub(super) instructions: String,
    pub(super) outcomes: Vec<LabelFile>,
    #[serde(default)]
    pub(super) evidence: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(untagged)]
pub(super) enum LabelFile {
    Value(String),
    Described { value: String, description: String },
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum AskRef {
    Named(String),
    Inline(AskFile),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StepFile {
    pub(super) id: String,
    pub(super) judge: String,
    pub(super) ask: AskRef,
}

#[derive(Deserialize)]
pub(super) struct ThenFile {
    #[serde(rename = "if", default)]
    pub(super) needs: BTreeMap<String, String>,
    #[serde(flatten)]
    pub(super) does: BTreeMap<String, Value>,
}
