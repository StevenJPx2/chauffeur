//! What a monitor follows, and the service that keeps monitors.

use serde::{Deserialize, Serialize};

/// Something an integration can follow for the agent.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Watch {
    /// A pull request, as `owner/repo` and its number.
    GithubPr { repo: String, number: u64 },
    /// A Jira issue, such as `ADEPT-123`.
    JiraIssue { key: String },
    /// A Slack thread, as its channel and parent message timestamp.
    SlackThread { channel: String, thread: String },
}

impl Watch {
    /// A short name for the monitor, such as `acme/app#42`.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::GithubPr { repo, number } => format!("{repo}#{number}"),
            Self::JiraIssue { key } => key.clone(),
            Self::SlackThread { channel, thread } => format!("slack {channel}/{thread}"),
        }
    }

    /// How a question describes it.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::GithubPr { repo, number } => format!("GitHub pull request {repo}#{number}"),
            Self::JiraIssue { key } => format!("Jira issue {key}"),
            Self::SlackThread { channel, thread } => {
                format!("Slack thread {thread} in channel {channel}")
            }
        }
    }
}

/// One monitor an integration keeps for an agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Monitor {
    pub id: String,
    /// `None` for a kind of monitor Chauffeur does not model, such as a
    /// Slack direct-message conversation.
    pub watch: Option<Watch>,
    pub enabled: bool,
}

/// The integration that keeps monitors, per agent. Calls block; an
/// unreachable integration returns an error and the capability stands aside.
pub trait Monitors: Send + Sync {
    fn list(&self, agent_id: &str) -> Result<Vec<Monitor>, String>;

    fn create(&self, agent_id: &str, watch: &Watch) -> Result<Monitor, String>;
}
