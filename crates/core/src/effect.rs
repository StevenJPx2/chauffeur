//! Typed effects a host applies at its seams.

use serde::{Deserialize, Serialize};

use crate::signal::ModelRef;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    /// Decision: answer the host's pending permission request.
    Permission {
        agent_id: String,
        decision: PermissionDecision,
        message: Option<String>,
    },
    /// Persistent enhancement: attach these skills to the user message being
    /// admitted, so their bodies enter history at the tail.
    AttachSkills {
        agent_id: String,
        skills: Vec<String>,
    },
    /// Persistent enhancement: a reminder delivered into the session so an
    /// idle agent resumes with it.
    Remind {
        agent_id: String,
        rule_id: String,
        text: String,
    },
    /// Persistent enhancement: steer the running turn after a tool call that
    /// likely broke the tool's best practice.
    Nudge {
        agent_id: String,
        tool: String,
        text: String,
    },
    /// Base decision: tools to hide for this context. A hide list, so a tool
    /// the engine never judged is never removed.
    HideTools {
        agent_id: String,
        tools: Vec<String>,
    },
    /// Persistent enhancement: point the agent at these Code Mode namespaces'
    /// tools for the user message being admitted, appended to that message.
    SurfaceTools {
        agent_id: String,
        namespaces: Vec<String>,
    },
    /// Show previously hidden tools again; the host's prompt cache is re-read
    /// once.
    RevealTools {
        agent_id: String,
        tools: Vec<String>,
    },
    /// Switch the agent to `model` and retry the failed request now.
    SwitchModel { agent_id: String, model: ModelRef },
    /// Keep the current model and let the host apply its own retry policy.
    KeepModel { agent_id: String },
    /// Decision: the integration event being gated should not reach the agent.
    WithholdEvent { agent_id: String },
}
