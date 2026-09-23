//! Git-specific steering rules.

use chauffeur_core::{Gate, Plugin, Rule};

pub struct GitPlugin;

impl Plugin for GitPlugin {
    fn id(&self) -> &'static str {
        "git"
    }

    fn rules(&self) -> Vec<Rule> {
        vec![Rule::new(
            "git:conflict-loop",
            "Git conflict loop detection",
            "The agent is stuck repeating git conflict-resolution commands without making progress.",
            "You appear stuck in a merge/rebase conflict loop. Abort the operation and escalate to a human instead of retrying.",
        )
        .gate(Gate::default().status(&["implementing"]))
        .priority(100)
        .once()
        .threshold(0.8)]
    }
}
