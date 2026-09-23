//! Jira-specific steering rules.

use chauffeur_core::{Gate, Plugin, Rule};

pub struct JiraPlugin;

impl Plugin for JiraPlugin {
    fn id(&self) -> &'static str {
        "jira"
    }

    fn rules(&self) -> Vec<Rule> {
        vec![
            Rule::new(
                "jira:transition-after-merge",
                "Transition Jira after PR merge",
                "The pull request has been merged.",
                "The PR has been merged. Transition the Jira ticket and assign it to the reporter.",
            )
            .gate(
                Gate::default()
                    .source(&["jira"])
                    .hooks(&["github:pr.merged"])
                    .tools_not_called(&["jira_transition_issue"]),
            )
            .priority(20)
            .once(),
        ]
    }
}
