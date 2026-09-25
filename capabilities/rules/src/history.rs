//! What each agent has done, as the exact facts rule gates check.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::rule::{Gate, History};

const MAX_AGENTS: usize = 256;
const MAX_TOOLS: usize = 256;
const MAX_HOOKS: usize = 64;

/// One agent's tools and integration events.
#[derive(Clone, Default, Deserialize, Serialize)]
struct Agent {
    /// The workspace of its latest tool call.
    workspace: String,
    /// Tools called since the latest user message, in `workspace`.
    turn: HashSet<String>,
    /// Tools called this session.
    session: HashSet<String>,
    /// Integration events this session, as `source:kind`.
    hooks: Vec<String>,
}

#[derive(Default, Deserialize, Serialize)]
pub struct Histories(HashMap<String, Agent>);

impl Histories {
    fn agent(&mut self, agent: &str) -> &mut Agent {
        if !self.0.contains_key(agent) && self.0.len() >= MAX_AGENTS {
            self.0.clear();
        }

        self.0.entry(agent.to_string()).or_default()
    }

    /// A new user message starts a new turn window.
    pub fn user_message(&mut self, agent: &str) {
        if let Some(history) = self.0.get_mut(agent) {
            history.turn.clear();
        }
    }

    /// A call in another workspace starts that workspace's turn window.
    pub fn tool(&mut self, agent: &str, workspace: &str, tool: &str) {
        let history = self.agent(agent);

        if history.workspace != workspace {
            history.workspace = workspace.to_string();
            history.turn.clear();
        }
        for tools in [&mut history.turn, &mut history.session] {
            if tools.len() < MAX_TOOLS {
                tools.insert(tool.to_string());
            }
        }
    }

    pub fn hook(&mut self, agent: &str, hook: String) {
        let history = self.agent(agent);

        if history.hooks.len() < MAX_HOOKS && !history.hooks.contains(&hook) {
            history.hooks.push(hook);
        }
    }

    /// Whether `gate` admits `agent` in `workspace`.
    #[must_use]
    pub fn admits(&self, gate: &Gate, agent: &str, workspace: &str) -> bool {
        let history = self.0.get(agent);
        let none = HashSet::new();
        let tools = match (gate.history, history) {
            (History::Turn, Some(history)) if history.workspace == workspace => &history.turn,
            (History::Session, Some(history)) => &history.session,
            _ => &none,
        };
        let hooks = history.map_or(&[][..], |history| history.hooks.as_slice());
        let (status, source) = derive(history.map_or(&none, |history| &history.session), hooks);
        let called = |tool: &String| tools.contains(tool);
        let one_of = |allowed: &[String], value: &str| {
            allowed.is_empty() || allowed.iter().any(|item| item == value)
        };

        gate.tools_called.iter().all(called)
            && (gate.tools_called_any.is_empty() || gate.tools_called_any.iter().any(called))
            && !gate.tools_not_called.iter().any(called)
            && one_of(&gate.status, status)
            && one_of(&gate.source, source)
            && (gate.hooks.is_empty() || gate.hooks.iter().any(|hook| hooks.contains(hook)))
    }

    /// Restore saved histories within bounds.
    pub fn restore(saved: Self) -> Self {
        Self(
            saved
                .0
                .into_iter()
                .take(MAX_AGENTS)
                .map(|(agent, mut history)| {
                    history.turn = history.turn.into_iter().take(MAX_TOOLS).collect();
                    history.session = history.session.into_iter().take(MAX_TOOLS).collect();
                    history.hooks.truncate(MAX_HOOKS);

                    (agent, history)
                })
                .collect(),
        )
    }
}

/// Status and source from the session's tools and events. A PR opened, or
/// any GitHub event, means `in_review`. Agents often open PRs and read Jira
/// through the shell, so events count as much as tools; Jira wins the source.
fn derive(tools: &HashSet<String>, hooks: &[String]) -> (&'static str, &'static str) {
    let any_tool = |prefix: &str| tools.iter().any(|tool| tool.starts_with(prefix));
    let any_hook = |prefix: &str| hooks.iter().any(|hook| hook.starts_with(prefix));
    let status = if tools.contains("github_open_pr") || any_hook("github:") {
        "in_review"
    } else {
        "implementing"
    };
    let source = if any_tool("jira_") || any_hook("jira:") {
        "jira"
    } else if any_tool("github_") || any_hook("github:") {
        "github"
    } else {
        ""
    };

    (status, source)
}
