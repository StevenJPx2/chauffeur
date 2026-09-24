//! Idle-reminder rules: a structural gate, a fuzzy situation for System One,
//! and a templated reminder. Plugins contribute them.

use std::collections::HashSet;

use serde::Deserialize;

pub const DEFAULT_THRESHOLD: f32 = 0.7;
pub const DEFAULT_COOLDOWN_SECS: u64 = 30;

/// Exact preconditions checked before the model is consulted. Structural
/// facts live here because the decision model cannot see them reliably:
/// measured, it scored "the agent opened a pull request" at 0.24 on a state
/// that literally contained `github_open_pr`. Empty lists mean "no constraint".
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Gate {
    #[serde(default)]
    pub status: Vec<String>,
    #[serde(default)]
    pub source: Vec<String>,
    /// At least one of these hooks must have fired.
    #[serde(default)]
    pub hooks: Vec<String>,
    /// Every one of these tools must appear in the history.
    #[serde(default)]
    pub tools_called: Vec<String>,
    /// At least one of these tools must appear in the history.
    #[serde(default)]
    pub tools_called_any: Vec<String>,
    /// None of these tools may appear in the history.
    #[serde(default)]
    pub tools_not_called: Vec<String>,
}

/// Structural facts about an agent that rule gates check.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IdleFacts {
    pub status: String,
    pub source: String,
    pub hooks: Vec<String>,
    pub tools_called: HashSet<String>,
}

impl IdleFacts {
    /// Derive status and source from the tools an agent has run: a PR opened
    /// means `in_review`; Jira or GitHub tools set the source.
    #[must_use]
    pub fn from_tools(tools_called: HashSet<String>) -> Self {
        let status = if tools_called.contains("github_open_pr") {
            "in_review"
        } else {
            "implementing"
        };
        let source = if tools_called.iter().any(|tool| tool.starts_with("jira_")) {
            "jira"
        } else if tools_called.iter().any(|tool| tool.starts_with("github_")) {
            "github"
        } else {
            ""
        };

        Self {
            status: status.into(),
            source: source.into(),
            hooks: Vec::new(),
            tools_called,
        }
    }

    /// Add integration events as `source:kind` hooks. Agents often open PRs
    /// and read Jira through the shell, so events count too: any GitHub event
    /// means the pull request exists (`in_review`), and any Jira event means a
    /// Jira source.
    #[must_use]
    pub fn with_hooks(mut self, hooks: Vec<String>) -> Self {
        if hooks.iter().any(|hook| hook.starts_with("github:")) {
            self.status = "in_review".into();

            if self.source.is_empty() {
                self.source = "github".into();
            }
        }
        if hooks.iter().any(|hook| hook.starts_with("jira:")) {
            self.source = "jira".into();
        }

        self.hooks = hooks;
        self
    }

    fn has_called(&self, tool: &str) -> bool {
        self.tools_called.contains(tool)
    }
}

impl Gate {
    pub fn admits(&self, context: &IdleFacts) -> bool {
        let status_ok = self.status.is_empty() || self.status.contains(&context.status);
        let source_ok = self.source.is_empty() || self.source.contains(&context.source);
        let hooks_ok =
            self.hooks.is_empty() || self.hooks.iter().any(|hook| context.hooks.contains(hook));
        let called_ok = self
            .tools_called
            .iter()
            .all(|tool| context.has_called(tool))
            && (self.tools_called_any.is_empty()
                || self
                    .tools_called_any
                    .iter()
                    .any(|tool| context.has_called(tool)));
        let not_called_ok = !self
            .tools_not_called
            .iter()
            .any(|tool| context.has_called(tool));

        status_ok && source_ok && hooks_ok && called_ok && not_called_ok
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub name: String,
    /// The fuzzy question the decision model answers as a proposition,
    /// e.g. "The agent has edited code but has not opened a pull request."
    pub situation: String,
    #[serde(default)]
    pub gate: Gate,
    /// Deterministic reminder text delivered to the agent.
    pub reminder: String,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub once: bool,
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
    /// Minimum P(true) from the model for the situation to count as present.
    #[serde(default = "default_threshold")]
    pub threshold: Threshold,
}

impl Rule {
    /// Builder entry point for plugins. Defaults: no gate, priority 0, not
    /// once, default cooldown and threshold.
    pub fn new(id: &str, name: &str, situation: &str, reminder: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            situation: situation.into(),
            gate: Gate::default(),
            reminder: reminder.into(),
            priority: 0,
            once: false,
            cooldown_secs: DEFAULT_COOLDOWN_SECS,
            threshold: Threshold(DEFAULT_THRESHOLD),
        }
    }

    #[must_use]
    pub fn gate(mut self, gate: Gate) -> Self {
        self.gate = gate;
        self
    }

    #[must_use]
    pub fn priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub fn once(mut self) -> Self {
        self.once = true;
        self
    }

    #[must_use]
    pub fn cooldown_secs(mut self, secs: u64) -> Self {
        self.cooldown_secs = secs;
        self
    }

    /// # Panics
    /// If `value` is outside `[0, 1]` — a programmer error in a plugin.
    #[must_use]
    pub fn threshold(mut self, value: f32) -> Self {
        self.threshold = Threshold::new(value).expect("plugin threshold must be within [0, 1]");
        self
    }
}

impl Gate {
    #[must_use]
    pub fn status(mut self, statuses: &[&str]) -> Self {
        self.status = statuses.iter().map(|s| s.to_string()).collect();
        self
    }

    #[must_use]
    pub fn source(mut self, sources: &[&str]) -> Self {
        self.source = sources.iter().map(|s| s.to_string()).collect();
        self
    }

    #[must_use]
    pub fn hooks(mut self, hooks: &[&str]) -> Self {
        self.hooks = hooks.iter().map(|s| s.to_string()).collect();
        self
    }

    #[must_use]
    pub fn tools_called(mut self, tools: &[&str]) -> Self {
        self.tools_called = tools.iter().map(|s| s.to_string()).collect();
        self
    }

    #[must_use]
    pub fn tools_called_any(mut self, tools: &[&str]) -> Self {
        self.tools_called_any = tools.iter().map(|s| s.to_string()).collect();
        self
    }

    #[must_use]
    pub fn tools_not_called(mut self, tools: &[&str]) -> Self {
        self.tools_not_called = tools.iter().map(|s| s.to_string()).collect();
        self
    }
}

/// A probability threshold in `[0, 1]`, validated at load.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Threshold(f32);

impl Threshold {
    pub fn new(value: f32) -> Result<Self, String> {
        if (0.0..=1.0).contains(&value) {
            Ok(Self(value))
        } else {
            Err(format!("threshold {value} is outside [0, 1]"))
        }
    }

    pub fn accepts(self, probability: f32) -> bool {
        probability >= self.0
    }
}

impl Eq for Threshold {}

impl<'de> Deserialize<'de> for Threshold {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = f32::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn default_threshold() -> Threshold {
    Threshold(DEFAULT_THRESHOLD)
}

fn default_cooldown() -> u64 {
    DEFAULT_COOLDOWN_SECS
}

pub fn load_rules(json: &str) -> Result<Vec<Rule>, String> {
    let rules: Vec<Rule> =
        serde_json::from_str(json).map_err(|error| format!("invalid rules: {error}"))?;

    if rules.is_empty() {
        return Err("rules file defines no rules".to_string());
    }

    let mut seen = std::collections::HashSet::new();

    for rule in &rules {
        if !seen.insert(rule.id.as_str()) {
            return Err(format!("duplicate rule id {}", rule.id));
        }
    }

    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_load_with_defaults_and_reject_duplicates() {
        let json = r#"[
          {"id":"a","name":"A","situation":"s","reminder":"r"},
          {"id":"b","name":"B","situation":"s","reminder":"r","priority":5,"once":true,"threshold":0.8}
        ]"#;

        let rules = load_rules(json).expect("valid rules");

        assert_eq!(rules[0].threshold, Threshold(DEFAULT_THRESHOLD));
        assert_eq!(rules[0].cooldown_secs, DEFAULT_COOLDOWN_SECS);
        assert!(rules[1].once);
        assert!(load_rules(r#"[{"id":"a","name":"A","situation":"s","reminder":"r"},{"id":"a","name":"A","situation":"s","reminder":"r"}]"#).is_err());
    }

    #[test]
    fn gate_checks_structural_tool_facts() {
        let context = IdleFacts::from_tools(HashSet::from(["edit".to_string()]));
        let no_pr_yet = Gate {
            tools_called: vec!["edit".into()],
            tools_not_called: vec!["github_open_pr".into()],
            ..Gate::default()
        };
        let pr_exists = Gate {
            tools_called: vec!["github_open_pr".into()],
            ..Gate::default()
        };

        assert_eq!(context.status, "implementing");
        assert!(no_pr_yet.admits(&context));
        assert!(!pr_exists.admits(&context));
    }

    #[test]
    fn facts_derive_status_and_source_from_tools() {
        let facts = IdleFacts::from_tools(HashSet::from([
            "github_open_pr".to_string(),
            "jira_view_issue".to_string(),
        ]));

        assert_eq!(
            (facts.status.as_str(), facts.source.as_str()),
            ("in_review", "jira")
        );
    }

    #[test]
    fn threshold_outside_unit_interval_is_rejected() {
        assert!(
            load_rules(r#"[{"id":"a","name":"A","situation":"s","reminder":"r","threshold":1.5}]"#)
                .is_err()
        );
    }
}
