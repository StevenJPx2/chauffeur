//! GitHub-specific steering rules.

use chauffeur_core::{Gate, Plugin, Rule};

pub struct GitHubPlugin;

impl Plugin for GitHubPlugin {
    fn id(&self) -> &'static str {
        "github"
    }

    fn rules(&self) -> Vec<Rule> {
        vec![create_pr(), fix_ci(), address_review()]
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

fn fix_ci() -> Rule {
    Rule::new(
        "github:fix-ci",
        "Fix CI failures",
        "There is an unread notification saying CI checks failed.",
        "CI checks are failing on your PR. Read the failing check, fix the cause, and push.",
    )
    .gate(Gate::default().status(&["in_review"]))
    .priority(25)
    .once()
}

fn address_review() -> Rule {
    Rule::new(
        "github:address-review",
        "Address PR review comments",
        "There is an unread notification that a reviewer requested changes on the pull request.",
        "A reviewer requested changes on your PR. Address the feedback and push updates.",
    )
    .gate(Gate::default().status(&["in_review"]))
    .priority(20)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_owns_three_prefixed_rules() {
        let rules = GitHubPlugin.rules();

        assert_eq!(rules.len(), 3);
        assert!(rules.iter().all(|rule| rule.id.starts_with("github:")));
    }
}
