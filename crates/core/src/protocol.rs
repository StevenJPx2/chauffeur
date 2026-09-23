//! Stable JSON contracts shared by the daemon, CLI, MCP, and host adapters.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::effect::Effect;
use crate::signal::Signal;

pub const METHOD_HEALTH: &str = "health";
/// Report one observation; returns the effects it produced.
pub const METHOD_SIGNAL: &str = "signal";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct DaemonRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct DaemonResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignalParams {
    pub signal: Signal,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SignalResult {
    pub effects: Vec<Effect>,
}
