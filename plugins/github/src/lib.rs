//! GitHub-specific steering rules.

use chauffeur_core::{Gate, Plugin, Rule};

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
    .gate(Gate::default().status(&["implementing"]).tools_not_called(&["github_open_pr"]))
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
}
