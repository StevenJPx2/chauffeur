//! Stable JSON contracts shared by daemon, CLI, MCP, and adapters.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::context::AgentContext;
use crate::queue::QueuedReminder;
use crate::skill::{SkillContext, SkillResult};
use crate::steer::Reminder;
use crate::target::Target;

pub const METHOD_STEER: &str = "steer";
pub const METHOD_REMINDERS_READ: &str = "reminders.read";
pub const METHOD_REMINDERS_ACKNOWLEDGE: &str = "reminders.acknowledge";
pub const METHOD_HEALTH: &str = "health";
pub const METHOD_SKILL_EVALUATE: &str = "skill.evaluate";
pub const METHOD_SKILLS_LIST: &str = "skills.list";

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

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EventFrame {
    Subscribed {
        target: Target,
    },
    Event {
        target: Target,
        reminders: Vec<QueuedReminder>,
    },
    Heartbeat,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SteerParams {
    pub target: Target,
    pub context: AgentContext,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct SteerResult {
    pub reminders: Vec<Reminder>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct TargetParams {
    pub target: Target,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AcknowledgeParams {
    pub target: Target,
    pub reminder_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SkillEvaluateParams {
    pub skill_id: String,
    pub context: SkillContext,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SkillEvaluateResult {
    pub evaluation: SkillResult,
}
