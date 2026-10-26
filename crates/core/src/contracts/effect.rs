//! Effects: the actions a host can take at its seams. They name what the host
//! does, never which capability asked, so a new capability needs no new
//! effect and no adapter change.

use serde::{Deserialize, Serialize};

use crate::signal::ModelRef;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
}

/// Where context enters the agent's conversation. Every delivery lands in
/// history at the tail, so the cached prefix stays intact.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// With the user message being admitted; only valid for that signal.
    Prompt,
    /// Into the running turn.
    Steer,
    /// As a message that wakes an idle agent.
    Resume,
    /// As a message the agent reads on its next turn.
    Wait,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    /// Answer the host's pending permission request.
    Permission {
        agent_id: String,
        decision: PermissionDecision,
        message: Option<String>,
    },
    /// Switch the agent to `model` and retry now, or with `None` keep it and
    /// let the host apply its own retry policy.
    Model {
        agent_id: String,
        model: Option<ModelRef>,
    },
    /// Omit or restore named direct tools in the model request, as judged by
    /// System One. Omission also makes them uncallable in that request; it
    /// does not revoke permissions or affect tools reached through Code Mode.
    Tools {
        agent_id: String,
        hide: Vec<String>,
        reveal: Vec<String>,
    },
    /// Withhold or restore whole Code Mode namespaces, by name. A withheld
    /// namespace is left out of the host's tool catalog and its search until
    /// restored; a namespace in neither list keeps its state.
    Namespaces {
        agent_id: String,
        hide: Vec<String>,
        reveal: Vec<String>,
    },
    /// Add skills (the host resolves their bodies) and text to the agent's
    /// context. `label` names the addition for the user.
    Context {
        agent_id: String,
        delivery: Delivery,
        label: String,
        skills: Vec<String>,
        text: Option<String>,
    },
    /// Whether the integration event being gated reaches the agent.
    Gate { agent_id: String, deliver: bool },
    /// Compact the agent's context before the prompt being admitted, with
    /// `model` writing the summary. The agent keeps its own model.
    Compact { agent_id: String, model: ModelRef },
    /// Keep the idle agent's prompt cache warm: a transient request on its
    /// context every `every_seconds`, at most `times` times. The agent's next
    /// request of its own ends the warming.
    Warm {
        agent_id: String,
        every_seconds: u64,
        times: u32,
    },
    /// The answer to a skill review: whether to add the skill.
    SkillVerdict {
        agent_id: String,
        safety: SkillSafety,
        /// Which judge decided: `system_one` or `system_two`; empty when none did.
        judge: String,
        /// Why, in a sentence; for `undetermined`, why no judge decided.
        reason: String,
        /// Each a cited file or section and what it asks the agent to do.
        concerns: Vec<String>,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SkillSafety {
    Safe,
    Unsafe,
    /// No judge reached a verdict; never read as safe.
    Undetermined,
}
