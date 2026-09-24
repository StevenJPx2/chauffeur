//! Normalized observations a host reports about an agent.

use serde::{Deserialize, Serialize};

pub const MAX_AGENT_ID_BYTES: usize = 128;
pub const MAX_TEXT_BYTES: usize = 2_048;
pub const MAX_AVAILABLE_MODELS: usize = 256;
pub const MAX_CATALOG_ENTRIES: usize = 128;
pub const MAX_DESCRIPTION_BYTES: usize = 400;
pub const MAX_RESOURCES: usize = 32;
pub const MAX_USER_REQUESTS: usize = 8;

/// A resource named in a permission request.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    /// As the agent requested it.
    pub requested: String,
    /// Absolute, with symlinks resolved where the path exists.
    pub resolved: String,
}

/// A skill or tool the host can expose, as the host names it.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub id: String,
    pub description: String,
    /// Size of the content exposing it would add, for per-turn budgets.
    pub bytes: u32,
}

/// Tools in a Code Mode namespace reach the model through `execute`, whose
/// catalog shows each namespace only in part.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CodeModeNamespace {
    pub name: String,
    /// Every tool in the namespace.
    pub size: u32,
    /// The host's best matches for the user's request, best first.
    pub tools: Vec<CatalogEntry>,
}

pub const MAX_CODE_MODE_NAMESPACES: usize = 64;
pub const MAX_CODE_MODE_MATCHES: usize = 8;

/// A model reference as `provider/model`, optionally at a thinking variant
/// (`provider/model#variant`), such as `anthropic/claude-opus-5-5#high`.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq, Hash)]
#[serde(deny_unknown_fields)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
    /// The host's thinking variant; `None` is the model's default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl ModelRef {
    #[must_use]
    pub fn key(&self) -> String {
        match &self.variant {
            Some(variant) => format!("{}/{}#{variant}", self.provider, self.model),
            None => format!("{}/{}", self.provider, self.model),
        }
    }
}

/// A model the host can switch to. Only enabled, active models are usable.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AvailableModel {
    pub model: ModelRef,
    pub usable: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SignalKind {
    /// A user prompt is being admitted.
    UserMessage {
        text: String,
        /// No earlier user message since the session started or last compacted.
        first_in_context: bool,
        /// Skills that could be attached, excluding ones already attached.
        skills: Vec<CatalogEntry>,
        /// With `first_in_context`, every tool the host offers; otherwise the
        /// tools currently hidden in this context.
        tools: Vec<CatalogEntry>,
        /// The session's current model, when the host knows it.
        #[serde(default)]
        model: Option<ModelRef>,
        /// The host's Code Mode namespaces, with each one's best matches for
        /// this message.
        #[serde(default)]
        code_mode: Vec<CodeModeNamespace>,
    },
    /// The agent asked to perform an action that needs permission.
    PermissionRequest {
        action: String,
        resources: Vec<Resource>,
        /// The agent's stated reason, if any.
        request: String,
        /// Absolute workspace root, symlinks resolved.
        workspace: String,
        /// Recent user messages in this context, oldest first.
        user_requests: Vec<String>,
    },
    ToolResult {
        tool: String,
        ok: bool,
        /// Clipped summary of the tool's input.
        #[serde(default)]
        input: String,
        /// Clipped error message when the call failed, such as a refusal.
        #[serde(default)]
        error: String,
    },
    /// The agent finished its turn and is idle.
    TurnEnd,
    /// A model request failed. The host reports every retryable error; core
    /// decides whether it is a usage limit.
    ModelError {
        model: ModelRef,
        error_type: String,
        status: Option<u16>,
        message: String,
        /// A tool ran during the failed step, so retrying could repeat it.
        tool_executed: bool,
        available: Vec<AvailableModel>,
    },
    ModelSucceeded {
        model: ModelRef,
    },
    /// An event from an integration watching the agent's work, such as a
    /// sourcefed monitor on its pull request or Jira issue.
    IntegrationEvent {
        /// The integration's source, such as `github`, `jira`, or `slack`.
        source: String,
        /// The event kind within that source, such as `ci` or `merged`.
        kind: String,
        summary: String,
        #[serde(default)]
        body: String,
        /// Whether the integration expects the agent to act on it.
        actionable: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Signal {
    pub agent_id: String,
    pub at: u64,
    pub kind: SignalKind,
}

impl Signal {
    /// Reject signals that exceed the bounds the engine relies on.
    pub fn validate(&self) -> Result<(), String> {
        if self.agent_id.is_empty() || self.agent_id.len() > MAX_AGENT_ID_BYTES {
            return Err(format!(
                "signal agent_id must contain 1 to {MAX_AGENT_ID_BYTES} bytes"
            ));
        }

        match &self.kind {
            SignalKind::ToolResult {
                tool, input, error, ..
            } => {
                bounded(tool, "tool")?;
                bounded(input, "tool input")?;
                bounded(error, "tool error")
            }
            SignalKind::TurnEnd => Ok(()),
            SignalKind::ModelError {
                model,
                error_type,
                message,
                available,
                ..
            } => {
                validate_model(model)?;
                bounded(error_type, "error_type")?;
                bounded(message, "message")?;

                if available.len() > MAX_AVAILABLE_MODELS {
                    return Err(format!(
                        "signal lists more than {MAX_AVAILABLE_MODELS} models"
                    ));
                }

                available
                    .iter()
                    .try_for_each(|entry| validate_model(&entry.model))
            }
            SignalKind::ModelSucceeded { model } => validate_model(model),
            SignalKind::IntegrationEvent {
                source,
                kind,
                summary,
                body,
                ..
            } => {
                if source.is_empty() || kind.is_empty() {
                    return Err("integration event source and kind must be non-empty".into());
                }

                bounded(source, "source")?;
                bounded(kind, "kind")?;
                bounded(summary, "summary")?;
                bounded(body, "body")
            }
            SignalKind::PermissionRequest {
                action,
                resources,
                request,
                workspace,
                user_requests,
            } => {
                bounded(action, "action")?;
                bounded(request, "request")?;
                bounded(workspace, "workspace")?;

                if resources.len() > MAX_RESOURCES || user_requests.len() > MAX_USER_REQUESTS {
                    return Err(format!(
                        "signal lists more than {MAX_RESOURCES} resources or {MAX_USER_REQUESTS} user requests"
                    ));
                }

                resources.iter().try_for_each(|resource| {
                    bounded(&resource.requested, "resource")?;
                    bounded(&resource.resolved, "resource")
                })?;
                user_requests
                    .iter()
                    .try_for_each(|text| bounded(text, "user request"))
            }
            SignalKind::UserMessage {
                text,
                skills,
                tools,
                model,
                code_mode,
                ..
            } => {
                bounded(text, "text")?;
                validate_code_mode(code_mode)?;

                if let Some(model) = model {
                    validate_model(model)?;
                }

                validate_catalog(skills, "skills")?;
                validate_catalog(tools, "tools")
            }
        }
    }
}

fn validate_code_mode(namespaces: &[CodeModeNamespace]) -> Result<(), String> {
    if namespaces.len() > MAX_CODE_MODE_NAMESPACES {
        return Err(format!(
            "signal lists more than {MAX_CODE_MODE_NAMESPACES} Code Mode namespaces"
        ));
    }

    namespaces.iter().try_for_each(|namespace| {
        if namespace.name.is_empty() || namespace.tools.len() > MAX_CODE_MODE_MATCHES {
            return Err(format!(
                "a Code Mode namespace needs a name and at most {MAX_CODE_MODE_MATCHES} matches"
            ));
        }

        bounded(&namespace.name, "Code Mode namespace")?;
        validate_catalog(&namespace.tools, "Code Mode tools")
    })
}

fn validate_catalog(entries: &[CatalogEntry], field: &str) -> Result<(), String> {
    if entries.len() > MAX_CATALOG_ENTRIES {
        return Err(format!(
            "signal lists more than {MAX_CATALOG_ENTRIES} {field}"
        ));
    }

    for entry in entries {
        if entry.id.is_empty() {
            return Err(format!("signal {field} entry id must be non-empty"));
        }

        bounded(&entry.id, field)?;

        if entry.description.len() > MAX_DESCRIPTION_BYTES {
            return Err(format!(
                "signal {field} description exceeds {MAX_DESCRIPTION_BYTES} bytes"
            ));
        }
    }

    Ok(())
}

fn validate_model(model: &ModelRef) -> Result<(), String> {
    if model.provider.is_empty() || model.model.is_empty() {
        return Err("model provider and id must be non-empty".into());
    }

    bounded(&model.provider, "model provider")?;
    bounded(&model.model, "model id")
}

fn bounded(value: &str, field: &str) -> Result<(), String> {
    if value.len() > MAX_TEXT_BYTES {
        return Err(format!("signal {field} exceeds {MAX_TEXT_BYTES} bytes"));
    }

    Ok(())
}
