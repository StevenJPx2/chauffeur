//! GitHub-specific steering rules.

use chauffeur_capability_idle_reminder::{Gate, Plugin, Rule};

pub struct GitHubPlugin;

impl Plugin for GitHubPlugin {
    fn id(&self) -> &'static str {
        "github"
    }

    fn rules(&self) -> Vec<Rule> {
        // CI failures and review requests reach the agent from sourcefed
        // itself; Chauffeur adds only follow-through sourcefed cannot know.
        vec![create_pr()]
    }
}

fn create_pr() -> Rule {
    Rule::new(
        "github:create-pr",
        "Create PR after implementation",
        "The agent has edited or written source code.",
        "You've made code changes but haven't created a PR yet. Run the tests, commit, push the branch, then open the PR.",
    )
    // Only after the agent changed files: the model alone judged a browser-only
    // session as "edited source code".
    .gate(
        Gate::default()
            .status(&["implementing"])
            .tools_called_any(&["edit", "write", "patch"])
            .tools_not_called(&["github_open_pr"]),
    )
    .priority(15)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_owns_prefixed_rules() {
        let rules = GitHubPlugin.rules();

        assert_eq!(rules.len(), 1);
        assert!(rules.iter().all(|rule| rule.id.starts_with("github:")));
    }

    #[test]
    fn create_pr_waits_for_a_file_change() {
        let gate = &create_pr().gate;
        let facts = |tools: &[&str]| {
            chauffeur_capability_idle_reminder::IdleFacts::from_tools(
                tools.iter().map(|tool| (*tool).to_string()).collect(),
            )
        };

        assert!(!gate.admits(&facts(&["execute", "read"])));
        assert!(gate.admits(&facts(&["read", "edit"])));
        assert!(!gate.admits(&facts(&["edit", "github_open_pr"])));
    }
}
