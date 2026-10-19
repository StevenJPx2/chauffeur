//! Stable JSON contracts shared by the daemon, CLI, MCP, and host adapters.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::effect::{Effect, SkillSafety};
use crate::signal::Signal;

pub const METHOD_HEALTH: &str = "health";
/// Report one observation; returns the effects it produced.
pub const METHOD_SIGNAL: &str = "signal";
/// The rulebooks a user can start, for a host to offer as commands.
pub const METHOD_RULEBOOKS: &str = "rulebooks";
/// The shared remote skill catalog, including source revision and refresh status.
pub const METHOD_SKILLS: &str = "skills";
/// Approve a remote skill's held upstream version: [`SkillAcceptParams`].
pub const METHOD_SKILL_ACCEPT: &str = "skill_accept";
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
    /// Upstream versions agents do not see yet, and why.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub held: Vec<HeldSkill>,
}

/// A remote skill whose latest version is not approved.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HeldSkill {
    pub id: String,
    pub source_id: String,
    /// The held upstream revision.
    pub revision: String,
    /// `None` while the judges have not answered.
    pub safety: Option<SkillSafety>,
    pub reason: String,
    pub concerns: Vec<String>,
    /// The approved revision agents see instead; `None` withholds the skill.
    pub serving: Option<String>,
}

/// Approve a remote skill's latest version: `{ "id": "SKILL" }`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkillAcceptParams {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SkillAcceptResult {
    pub id: String,
    pub source_id: String,
    pub revision: String,
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
pub struct SignalParams {
    pub signal: Signal,
    /// Remaining host time, including queueing; absent uses the daemon's limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SignalResult {
    pub effects: Vec<Effect>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signal_budgets_are_optional_and_additive_fields_are_compatible() {
        let wire = json!({ "signal": { "agent_id": "a", "at": 1, "kind": {
            "type": "turn_end", "workspace": "", "subagent": false,
            "user_request": "", "summary": "", "todos": []
        } } });
        let old: SignalParams = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(old.deadline_ms, None);
        assert_eq!(serde_json::to_value(&old).unwrap(), wire);
        let mut new = wire;
        new["deadline_ms"] = json!(2_000);
        new["future_field"] = json!(true);
        assert_eq!(
            serde_json::from_value::<SignalParams>(new)
                .unwrap()
                .deadline_ms,
            Some(2_000)
        );
    }
}
