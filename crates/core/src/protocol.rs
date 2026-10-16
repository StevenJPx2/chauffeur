//! Stable JSON contracts shared by the daemon, CLI, MCP, and host adapters.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::effect::Effect;
use crate::signal::Signal;

pub const METHOD_HEALTH: &str = "health";
/// Report one observation; returns the effects it produced.
pub const METHOD_SIGNAL: &str = "signal";
/// The rulebooks a user can start, for a host to offer as commands.
pub const METHOD_RULEBOOKS: &str = "rulebooks";
/// The shared remote skill catalog, including source revision and refresh status.
pub const METHOD_SKILLS: &str = "skills";
/// The wording a host shows the agent: `{ "host": "opencode" }` returns the
/// host's texts, shipped and overlaid by the user's file.
pub const METHOD_TEXTS: &str = "texts";
/// Stop the daemon once in-flight requests finish, so a host can replace a
/// daemon of another version with its own.
pub const METHOD_SHUTDOWN: &str = "shutdown";

/// One rulebook as a host offers it, such as the `/goal` command.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RulebookEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Starting it without arguments is refused.
    pub args_required: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RulebooksResult {
    pub rulebooks: Vec<RulebookEntry>,
}

/// One parsed skill, independent of any host's skill registry or message format.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct RemoteSkill {
    pub id: String,
    pub description: String,
    pub content: String,
    /// Immutable local snapshot path. Relative references resolve beside this file.
    pub path: String,
    pub source_id: String,
    pub source_url: String,
    pub revision: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SkillSourceState {
    Ready {
        revision: String,
        checked_at: u64,
    },
    Stale {
        revision: String,
        checked_at: u64,
        error: String,
    },
    Unavailable {
        error: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SkillSourceStatus {
    pub id: String,
    #[serde(flatten)]
    pub state: SkillSourceState,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct SkillsResult {
    pub skills: Vec<RemoteSkill>,
    pub sources: Vec<SkillSourceStatus>,
}

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
