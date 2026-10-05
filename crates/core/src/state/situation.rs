//! Bounded rolling view of one agent's recent activity.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::signal::{Signal, SignalKind};

pub const MAX_ENTRIES: usize = 32;
const MAX_LINE_CHARS: usize = 240;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Situation {
    entries: VecDeque<String>,
    last_at: u64,
}

impl Situation {
    pub fn record(&mut self, signal: &Signal) {
        self.last_at = self.last_at.max(signal.at);

        let Some(line) = describe(&signal.kind) else {
            return;
        };

        if self.entries.len() == MAX_ENTRIES {
            self.entries.pop_front();
        }

        self.entries.push_back(if quotes_user(&signal.kind) {
            line
        } else {
            clip(&line)
        });
    }

    #[must_use]
    pub fn last_at(&self) -> u64 {
        self.last_at
    }

    /// Prose rendering for System One: recent activity, oldest first.
    #[must_use]
    pub fn render(&self) -> String {
        if self.entries.is_empty() {
            return "No recent agent activity is recorded.".into();
        }

        let mut state = String::from("Recent agent activity, oldest first:\n");

        for entry in &self.entries {
            state.push_str("- ");
            state.push_str(entry);
            state.push('\n');
        }

        state
    }
}

fn describe(kind: &SignalKind) -> Option<String> {
    match kind {
        SignalKind::UserMessage { text, .. } => Some(format!("User: {text}")),
        SignalKind::AgentRequest { need, .. } => Some(format!("Agent asks Chauffeur for: {need}")),
        SignalKind::PermissionRequest {
            action,
            resources,
            request,
            workspace,
            user_requests,
            ..
        } => {
            let targets: Vec<&str> = resources
                .iter()
                .map(|resource| resource.resolved.as_str())
                .collect();
            // User intent decides permission; the latest request travels with it.
            let asked = user_requests.last().map_or("none recorded", String::as_str);
            // Where the session works, so a request can be judged against it.
            let project = if workspace.is_empty() {
                String::new()
            } else {
                format!(" from the project {workspace}")
            };
            let reason = if request.trim().is_empty() {
                "no reason given"
            } else {
                request.as_str()
            };

            Some(format!(
                "Agent asks to {action} {}{project}. Its reason: {reason} (latest user request: {asked})",
                targets.join(", ")
            ))
        }
        SignalKind::ToolResult {
            tool,
            ok,
            input,
            error,
            ..
        } => {
            let outcome = if *ok { "Ran tool" } else { "Tool failed:" };
            let mut line = if input.is_empty() {
                format!("{outcome} {tool}.")
            } else {
                format!("{outcome} {tool} {input}")
            };

            if !error.is_empty() {
                line.push_str(&format!(" (error: {error})"));
            }

            Some(line)
        }
        SignalKind::ModelError {
            model,
            error_type,
            message,
            ..
        } => Some(format!(
            "Model {} failed with {error_type}: {message}",
            model.key()
        )),
        SignalKind::IntegrationEvent {
            source,
            kind,
            summary,
            ..
        } => Some(format!("{source} {kind} event: {summary}")),
        SignalKind::TurnEnd { summary, .. } if !summary.trim().is_empty() => {
            Some(format!("Agent ended its turn: {summary}"))
        }
        SignalKind::Rulebook {
            command,
            rulebook,
            args,
            ..
        } => Some(format!("User ran rulebook {rulebook} {command:?}: {args}")),
        SignalKind::ModelSucceeded { .. } | SignalKind::TurnEnd { .. } => None,
    }
}

/// Lines carrying the user's own words are kept whole; the signal bounds
/// them. Lines about tools and models are clipped.
fn quotes_user(kind: &SignalKind) -> bool {
    matches!(
        kind,
        SignalKind::UserMessage { .. }
            | SignalKind::PermissionRequest { .. }
            | SignalKind::Rulebook { .. }
    )
}

fn clip(line: &str) -> String {
    line.chars().take(MAX_LINE_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(n: usize) -> Signal {
        Signal {
            agent_id: "a".into(),
            at: n as u64,
            kind: SignalKind::ToolResult {
                tool: format!("tool-{n}"),
                ok: true,
                workspace: String::new(),
                subagent: false,
                input: String::new(),
                error: String::new(),
                user_request: String::new(),
                evidence: String::new(),
                candidates: Vec::new(),
            },
        }
    }

    #[test]
    fn permission_requests_carry_the_latest_user_request() {
        let mut situation = Situation::default();

        situation.record(&Signal {
            agent_id: "a".into(),
            at: 1,
            kind: SignalKind::PermissionRequest {
                action: "edit".into(),
                resources: vec![crate::signal::Resource {
                    requested: "README.md".into(),
                    resolved: "/w/README.md".into(),
                }],
                request: "update install".into(),
                workspace: "/w".into(),
                user_requests: vec!["old".into(), "please update README.md".into()],
                host_decision: crate::effect::PermissionDecision::Ask,
            },
        });

        assert!(situation.render().contains(
            "Agent asks to edit /w/README.md from the project /w. Its reason: update install (latest user request: please update README.md)"
        ));
    }

    #[test]
    fn a_failed_tool_call_carries_its_error() {
        let mut situation = Situation::default();

        situation.record(&Signal {
            agent_id: "a".into(),
            at: 1,
            kind: SignalKind::ToolResult {
                tool: "edit".into(),
                ok: false,
                workspace: String::new(),
                subagent: false,
                input: r#"{"path":"main.rs"}"#.into(),
                error: "permission denied".into(),
                user_request: String::new(),
                evidence: String::new(),
                candidates: Vec::new(),
            },
        });

        assert!(
            situation
                .render()
                .contains(r#"Tool failed: edit {"path":"main.rs"} (error: permission denied)"#)
        );
    }

    #[test]
    fn keeps_only_the_newest_entries() {
        let mut situation = Situation::default();

        for n in 0..(MAX_ENTRIES + 5) {
            situation.record(&tool(n));
        }

        let state = situation.render();

        assert!(!state.contains("tool-4."));
        assert!(state.contains("tool-5."));
        assert!(state.contains(&format!("tool-{}.", MAX_ENTRIES + 4)));
        assert_eq!(situation.last_at(), (MAX_ENTRIES + 4) as u64);
    }

    #[test]
    fn user_words_stay_whole_while_tool_lines_are_clipped() {
        let mut situation = Situation::default();
        let words = format!("{} the end", "requirement ".repeat(100));

        situation.record(&Signal {
            agent_id: "a".into(),
            at: 1,
            kind: SignalKind::UserMessage {
                text: words.clone(),
                first_in_context: true,
                skills: Vec::new(),
                tools: Vec::new(),
                model: None,
                code_mode: Vec::new(),
                workspace: String::new(),
            },
        });
        let mut long_tool = tool(1);
        if let SignalKind::ToolResult { input, .. } = &mut long_tool.kind {
            *input = "x".repeat(1_000);
        }
        situation.record(&long_tool);

        let state = situation.render();

        assert!(state.contains(&format!("User: {words}")));
        assert!(!state.contains(&"x".repeat(MAX_LINE_CHARS)));
    }
}
