//! Normalized observations a host reports about an agent.

use serde::{Deserialize, Serialize};

use crate::effect::PermissionDecision;

pub const MAX_AGENT_ID_BYTES: usize = 128;
pub const MAX_TEXT_BYTES: usize = 2_048;
/// The user's own words: a message, a request quoted in another signal, or
/// a rulebook's arguments. Jev judged 64 KiB goals in under a second.
pub const MAX_PROMPT_BYTES: usize = 65_536;
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

/// Items a session's todo list may hold.
pub const MAX_TODOS: usize = 64;
/// Bytes one todo's text may hold.
pub const MAX_TODO_BYTES: usize = 512;

/// Where one todo stands.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl TodoStatus {
    /// Pending or in progress: work the agent still owes.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Pending | Self::InProgress)
    }
}

/// One item of the agent's todo list.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Todo {
    pub content: String,
    pub status: TodoStatus,
}

/// What a todo list says about the work, as a rule gate checks it.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TodoState {
    /// The agent keeps no todos.
    None,
    /// At least one todo is pending or in progress.
    Open,
    /// Every todo is completed or cancelled.
    Done,
}

impl TodoState {
    #[must_use]
    pub fn of(todos: &[Todo]) -> Self {
        if todos.is_empty() {
            Self::None
        } else if todos.iter().any(|todo| todo.status.is_open()) {
            Self::Open
        } else {
            Self::Done
        }
    }
}

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
        /// The session's working directory, so a judgment can match skills
        /// and tools to the project the request is about.
        #[serde(default)]
        workspace: String,
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
        /// What the host would decide on its own. Contracts judge only what
        /// it would ask about; the backstop sees every request.
        #[serde(default = "host_asks")]
        host_decision: PermissionDecision,
    },
    ToolResult {
        tool: String,
        ok: bool,
        /// The session's current workspace, for project-local contracts.
        #[serde(default)]
        workspace: String,
        /// The call ran in a subagent's session, not one the user drives.
        #[serde(default)]
        subagent: bool,
        /// Clipped summary of the tool's input.
        #[serde(default)]
        input: String,
        /// Clipped error message when the call failed, such as a refusal.
        #[serde(default)]
        error: String,
        /// Bounded recent user request, for recovering a missing tool.
        #[serde(default)]
        user_request: String,
        /// The call's error and output as the host reports them, clipped
        /// with both ends kept.
        #[serde(default)]
        evidence: String,
        /// Registered, currently hidden direct tools with local search matches.
        #[serde(default)]
        candidates: Vec<CatalogEntry>,
    },
    /// The agent asked, in its own words, for a tool or skill it lacks.
    AgentRequest {
        /// What the agent says it needs.
        need: String,
        /// The user's latest request.
        #[serde(default)]
        user_request: String,
        /// Direct tools currently hidden from the agent.
        #[serde(default)]
        tools: Vec<CatalogEntry>,
        /// The host's Code Mode namespaces, with each one's best matches for
        /// the need.
        #[serde(default)]
        code_mode: Vec<CodeModeNamespace>,
    },
    /// The agent finished its turn and is idle. The host supplies its current
    /// workspace and latest user request for scoped follow-through contracts.
    TurnEnd {
        #[serde(default)]
        workspace: String,
        /// The turn was a subagent's, not one the user drives.
        #[serde(default)]
        subagent: bool,
        #[serde(default)]
        user_request: String,
        /// The agent's closing message for the turn, clipped: evidence for
        /// judging whether work is done.
        #[serde(default)]
        summary: String,
        /// The session's todo list as the agent last wrote it.
        #[serde(default)]
        todos: Vec<Todo>,
    },
    /// The user started, paused, resumed, cleared, or asked about a rulebook
    /// in this session, such as `/goal <objective>`.
    Rulebook {
        command: RulebookCommand,
        rulebook: String,
        #[serde(default)]
        args: String,
        /// The session's working directory: where the rulebook must be in
        /// scope, and where a project's own rulebooks are found.
        #[serde(default)]
        workspace: String,
    },
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
        /// The integration's ID for the monitor that produced the event, when
        /// it has one, such as a sourcefed monitor ID.
        #[serde(default)]
        monitor: String,
    },
}

/// What the user asked of a rulebook.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RulebookCommand {
    Start,
    Pause,
    Resume,
    Clear,
    Status,
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

        self.kind.validate()
    }
}

impl SignalKind {
    fn validate(&self) -> Result<(), String> {
        match self {
            SignalKind::ToolResult {
                tool,
                workspace,
                input,
                error,
                user_request,
                evidence,
                candidates,
                ..
            } => {
                all_bounded(&[
                    (tool, "tool"),
                    (workspace, "workspace"),
                    (input, "tool input"),
                    (error, "tool error"),
                    (evidence, "tool evidence"),
                ])?;
                prompt_bounded(user_request, "user request")?;
                validate_catalog(candidates, "tool candidates")
            }
            SignalKind::TurnEnd {
                workspace,
                user_request,
                summary,
                todos,
                ..
            } => {
                all_bounded(&[(workspace, "workspace"), (summary, "summary")])?;
                prompt_bounded(user_request, "user request")?;
                validate_todos(todos)
            }
            SignalKind::Rulebook {
                rulebook,
                args,
                workspace,
                ..
            } => {
                if rulebook.is_empty() {
                    return Err("a rulebook signal must name its rulebook".into());
                }

                all_bounded(&[(rulebook, "rulebook"), (workspace, "workspace")])?;
                prompt_bounded(args, "rulebook args")
            }
            SignalKind::AgentRequest {
                need,
                user_request,
                tools,
                code_mode,
            } => validate_agent_request(need, user_request, tools, code_mode),
            SignalKind::ModelError {
                model,
                error_type,
                message,
                available,
                ..
            } => validate_model_error(model, error_type, message, available),
            SignalKind::ModelSucceeded { model } => validate_model(model),
            SignalKind::IntegrationEvent { .. } => validate_integration_event(self),
            SignalKind::PermissionRequest {
                action,
                resources,
                request,
                workspace,
                user_requests,
                ..
            } => {
                all_bounded(&[
                    (action, "action"),
                    (request, "request"),
                    (workspace, "workspace"),
                ])?;
                validate_permission_lists(resources, user_requests)
            }
            SignalKind::UserMessage { .. } => validate_user_message(self),
        }
    }
}

/// Hosts that predate `host_decision` sent only requests they would ask about
/// or allow; judging both as asks keeps their contracts working.
fn host_asks() -> PermissionDecision {
    PermissionDecision::Ask
}

fn validate_agent_request(
    need: &str,
    user_request: &str,
    tools: &[CatalogEntry],
    code_mode: &[CodeModeNamespace],
) -> Result<(), String> {
    if need.trim().is_empty() {
        return Err("an agent request must say what it needs".into());
    }

    bounded(need, "need")?;
    prompt_bounded(user_request, "user request")?;
    validate_code_mode(code_mode)?;
    validate_catalog(tools, "tools")
}

fn validate_integration_event(event: &SignalKind) -> Result<(), String> {
    let SignalKind::IntegrationEvent {
        source,
        kind,
        summary,
        body,
        monitor,
        ..
    } = event
    else {
        return Ok(());
    };

    if source.is_empty() || kind.is_empty() {
        return Err("integration event source and kind must be non-empty".into());
    }

    bounded(source, "source")?;
    bounded(kind, "kind")?;
    bounded(summary, "summary")?;
    bounded(monitor, "monitor")?;
    bounded(body, "body")
}

fn validate_user_message(message: &SignalKind) -> Result<(), String> {
    let SignalKind::UserMessage {
        text,
        skills,
        tools,
        model,
        code_mode,
        workspace,
        ..
    } = message
    else {
        return Ok(());
    };

    prompt_bounded(text, "text")?;
    bounded(workspace, "workspace")?;
    validate_code_mode(code_mode)?;

    if let Some(model) = model {
        validate_model(model)?;
    }

    validate_catalog(skills, "skills")?;
    validate_catalog(tools, "tools")
}

fn validate_model_error(
    model: &ModelRef,
    error_type: &str,
    message: &str,
    available: &[AvailableModel],
) -> Result<(), String> {
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

fn validate_permission_lists(
    resources: &[Resource],
    user_requests: &[String],
) -> Result<(), String> {
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
        .try_for_each(|text| prompt_bounded(text, "user request"))
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

/// Each `(value, field)` within the text bound.
fn validate_todos(todos: &[Todo]) -> Result<(), String> {
    if todos.len() > MAX_TODOS {
        return Err(format!("signal todos exceed {MAX_TODOS} items"));
    }

    if todos
        .iter()
        .any(|todo| todo.content.trim().is_empty() || todo.content.len() > MAX_TODO_BYTES)
    {
        return Err(format!("a todo must hold 1 to {MAX_TODO_BYTES} bytes"));
    }

    Ok(())
}

fn all_bounded(fields: &[(&String, &str)]) -> Result<(), String> {
    fields
        .iter()
        .try_for_each(|(value, field)| bounded(value, field))
}

fn bounded(value: &str, field: &str) -> Result<(), String> {
    if value.len() > MAX_TEXT_BYTES {
        return Err(format!("signal {field} exceeds {MAX_TEXT_BYTES} bytes"));
    }

    Ok(())
}

/// The user's own words, which hosts send whole: never clipped, only
/// bounded, at a size Jev judges in under a second.
fn prompt_bounded(value: &str, field: &str) -> Result<(), String> {
    if value.len() > MAX_PROMPT_BYTES {
        return Err(format!("signal {field} exceeds {MAX_PROMPT_BYTES} bytes"));
    }

    Ok(())
}
