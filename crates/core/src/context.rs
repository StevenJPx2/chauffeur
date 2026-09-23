//! What Chauffeur knows about an agent at the moment it goes idle.

use serde::{Deserialize, Serialize};

/// Tool calls kept in the compact brief sent to the decision model.
pub const BRIEF_TOOL_CALLS: usize = 12;
/// Notifications kept in the compact brief.
pub const BRIEF_NOTIFICATIONS: usize = 6;
/// Characters kept per free-text field in the brief.
pub const BRIEF_TEXT_LENGTH: usize = 120;

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub summary: String,
    /// Unix seconds.
    pub at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct Notice {
    pub source: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub acknowledged: bool,
}

/// One idle snapshot for one agent. This is the ingress record.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
pub struct AgentContext {
    pub agent_id: String,
    pub status: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub tool_history: Vec<ToolCall>,
    #[serde(default)]
    pub notifications: Vec<Notice>,
    /// Hook names seen since the last reminder, e.g. `github:pr.merged`.
    #[serde(default)]
    pub hooks: Vec<String>,
    /// Unix seconds when the agent went idle.
    pub idle_at: u64,
}

/// The bounded state handed to the decision model. Laya's context is 512
/// tokens, so the brief keeps only the newest, shortest evidence.
#[derive(Clone, Debug, Serialize, Eq, PartialEq)]
pub struct Brief {
    pub status: String,
    pub source: String,
    pub recent_tools: Vec<String>,
    pub unread: Vec<String>,
    pub hooks: Vec<String>,
}

impl Brief {
    /// Laya was trained on natural-language states; measured against the same
    /// snapshots, JSON states scored propositions near chance while prose
    /// separated them cleanly. Render as prose, never as JSON.
    pub fn to_prose(&self) -> String {
        let mut prose = format!("Agent status: {}.", self.status);

        if !self.source.is_empty() {
            prose.push_str(&format!(" Work item source: {}.", self.source));
        }

        if self.recent_tools.is_empty() {
            prose.push_str(" Recent actions: none.");
        } else {
            prose.push_str(&format!(
                " Recent actions, oldest first: {}.",
                self.recent_tools.join("; ")
            ));
        }

        if self.unread.is_empty() {
            prose.push_str(" Unread notifications: none.");
        } else {
            prose.push_str(&format!(
                " Unread notifications: {}.",
                self.unread.join("; ")
            ));
        }

        if !self.hooks.is_empty() {
            prose.push_str(&format!(" Recent events: {}.", self.hooks.join(", ")));
        }

        prose
    }
}

impl AgentContext {
    pub fn is_blocked(&self) -> bool {
        self.status == "blocked"
    }

    pub fn has_called(&self, tool: &str) -> bool {
        self.tool_history.iter().any(|call| call.name == tool)
    }

    pub fn brief(&self) -> Brief {
        let recent_tools = self
            .tool_history
            .iter()
            .rev()
            .take(BRIEF_TOOL_CALLS)
            // Verb phrases, not `name: args`: on the same conflict-loop
            // snapshot Laya scored "stuck" 0.48 for `bash: git rebase main`
            // and 0.89 for `ran bash (git rebase main)`.
            .map(|call| {
                if call.summary.is_empty() {
                    format!("ran {}", call.name)
                } else {
                    format!("ran {} ({})", call.name, clip(&call.summary))
                }
            })
            .collect::<Vec<_>>();

        let unread = self
            .notifications
            .iter()
            .filter(|notice| !notice.acknowledged)
            .rev()
            .take(BRIEF_NOTIFICATIONS)
            .map(|notice| format!("[{}] {}", notice.source, clip(&notice.title)))
            .collect::<Vec<_>>();

        Brief {
            status: self.status.clone(),
            source: self.source.clone(),
            recent_tools: recent_tools.into_iter().rev().collect(),
            unread: unread.into_iter().rev().collect(),
            hooks: self.hooks.clone(),
        }
    }
}

fn clip(text: &str) -> String {
    let trimmed = text.trim();

    if trimmed.chars().count() <= BRIEF_TEXT_LENGTH {
        return trimmed.to_string();
    }

    let mut clipped: String = trimmed.chars().take(BRIEF_TEXT_LENGTH - 1).collect();
    clipped.push('…');

    clipped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_with_tools(count: usize) -> AgentContext {
        AgentContext {
            agent_id: "a1".into(),
            status: "implementing".into(),
            source: "jira".into(),
            tool_history: (0..count)
                .map(|index| ToolCall {
                    name: format!("tool{index}"),
                    summary: String::new(),
                    at: index as u64,
                })
                .collect(),
            notifications: Vec::new(),
            hooks: Vec::new(),
            idle_at: 100,
        }
    }

    #[test]
    fn brief_keeps_only_the_newest_tool_calls_in_order() {
        let brief = context_with_tools(30).brief();

        assert_eq!(brief.recent_tools.len(), BRIEF_TOOL_CALLS);
        assert_eq!(
            brief.recent_tools.first().map(String::as_str),
            Some("ran tool18")
        );
        assert_eq!(
            brief.recent_tools.last().map(String::as_str),
            Some("ran tool29")
        );
    }

    #[test]
    fn brief_drops_acknowledged_notifications_and_clips_titles() {
        let mut context = context_with_tools(0);
        context.notifications = vec![
            Notice {
                source: "github".into(),
                title: "x".repeat(500),
                body: String::new(),
                acknowledged: false,
            },
            Notice {
                source: "github".into(),
                title: "seen".into(),
                body: String::new(),
                acknowledged: true,
            },
        ];

        let brief = context.brief();

        assert_eq!(brief.unread.len(), 1);
        assert!(brief.unread[0].chars().count() <= BRIEF_TEXT_LENGTH + "[github] ".len());
        assert!(brief.unread[0].ends_with('…'));
    }

    #[test]
    fn prose_names_every_evidence_section() {
        let mut context = context_with_tools(2);
        context.notifications.push(Notice {
            source: "github".into(),
            title: "CI check failed".into(),
            body: String::new(),
            acknowledged: false,
        });
        context.hooks.push("github:pr.merged".into());

        let prose = context.brief().to_prose();

        assert_eq!(
            prose,
            "Agent status: implementing. Work item source: jira. Recent actions, oldest first: ran tool0; ran tool1. \
             Unread notifications: [github] CI check failed. Recent events: github:pr.merged."
        );
        assert!(
            context_with_tools(0)
                .brief()
                .to_prose()
                .contains("Recent actions: none.")
        );
    }
}
