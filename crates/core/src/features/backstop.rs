//! The irreversible-harm backstop: the one deterministic gate outside System
//! One. It never routes; it vetoes a permission request whose facts match a
//! pattern, and hands one matching a `confirm` pattern to the user. Patterns
//! come from the shipped defaults (`skills/safety/backstop.json`), your
//! `backstop.json`, which adds to or replaces them, and commands System One
//! confidently judged irreversible.

use std::path::Path;

use serde::Deserialize;

use crate::config::load_config;
use crate::signal::{Signal, SignalKind};

/// The shipped defaults, compiled in.
const DEFAULTS: &str = include_str!("../../../../skills/safety/backstop.json");
const MAX_PATTERNS: usize = 128;
const MAX_LEARNED: usize = 512;
const MAX_PATTERN_BYTES: usize = 256;
/// A learned command must be at least this long, so a short, generic command
/// never becomes a pattern that blocks everything.
const MIN_LEARNED_BYTES: usize = 8;

/// Strict JSON config: `{"replace": false, "patterns": ["mkfs"], "confirm": ["origin +main"]}`.
/// Case-insensitive substrings; your file adds to the defaults, or with
/// `replace` stands in for them.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BackstopConfig {
    #[serde(default)]
    pub replace: bool,
    /// Requests these match are denied.
    #[serde(default)]
    pub patterns: Vec<String>,
    /// Requests these match always ask the user, whatever the host or a
    /// contract would decide. Each must end at a word boundary.
    #[serde(default)]
    pub confirm: Vec<String>,
}

pub struct Backstop {
    patterns: Vec<String>,
    confirm: Vec<String>,
    learned: Vec<String>,
}

impl Backstop {
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::from_config(load_config(path)?)
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The defaults plus (or replaced by) the config's patterns, within bounds.
    pub fn from_config(config: BackstopConfig) -> Result<Self, String> {
        let defaults: BackstopConfig =
            serde_json::from_str(DEFAULTS).map_err(|error| error.to_string())?;
        let (patterns, confirm) = if config.replace {
            (config.patterns, config.confirm)
        } else {
            (
                [defaults.patterns, config.patterns].concat(),
                [defaults.confirm, config.confirm].concat(),
            )
        };

        Ok(Self {
            patterns: bounded(patterns)?,
            confirm: bounded(confirm)?,
            learned: Vec::new(),
        })
    }

    /// The defaults plus `extra`.
    #[must_use]
    pub fn new(extra: Vec<String>) -> Self {
        Self::from_config(BackstopConfig {
            replace: false,
            patterns: extra,
            confirm: Vec::new(),
        })
        .unwrap_or_else(|_| {
            Self::from_config(BackstopConfig::default())
                .expect("shipped backstop defaults are valid")
        })
    }

    /// Commands learned earlier; kept apart so they can be reviewed.
    #[must_use]
    pub fn with_learned(mut self, learned: Vec<String>) -> Self {
        self.learned = learned
            .into_iter()
            .filter(|pattern| {
                pattern.len() >= MIN_LEARNED_BYTES && pattern.len() <= MAX_PATTERN_BYTES
            })
            .take(MAX_LEARNED)
            .collect();
        self
    }

    #[must_use]
    pub fn learned(&self) -> &[String] {
        &self.learned
    }

    /// Learn a command judged irreversible; `true` when it was new.
    pub fn learn(&mut self, command: &str) -> bool {
        let pattern = command.trim().to_lowercase();

        if pattern.len() < MIN_LEARNED_BYTES
            || pattern.len() > MAX_PATTERN_BYTES
            || self.learned.len() >= MAX_LEARNED
            || self.learned.contains(&pattern)
        {
            return false;
        }

        self.learned.push(pattern);
        true
    }

    /// The deny pattern a permission request matches, if any.
    #[must_use]
    pub fn veto(&self, signal: &Signal) -> Option<&str> {
        let facts = facts(signal)?;

        self.patterns
            .iter()
            .chain(&self.learned)
            .find(|pattern| matches(&facts, pattern, is_rooted(pattern)))
            .map(String::as_str)
    }

    /// The confirm pattern a permission request matches, if any.
    #[must_use]
    pub fn confirm(&self, signal: &Signal) -> Option<&str> {
        let facts = facts(signal)?;

        self.confirm
            .iter()
            .find(|pattern| matches(&facts, pattern, true))
            .map(String::as_str)
    }
}

/// Lowercased patterns, when every one is non-empty and within bounds.
fn bounded(patterns: Vec<String>) -> Result<Vec<String>, String> {
    if patterns.len() > MAX_PATTERNS
        || patterns
            .iter()
            .any(|pattern| pattern.trim().is_empty() || pattern.len() > MAX_PATTERN_BYTES)
    {
        return Err(format!(
            "at most {MAX_PATTERNS} non-empty patterns of at most {MAX_PATTERN_BYTES} bytes"
        ));
    }

    Ok(patterns
        .into_iter()
        .map(|pattern| pattern.to_lowercase())
        .collect())
}

/// A permission request's action, reason, and requested resources, lowercased.
fn facts(signal: &Signal) -> Option<String> {
    let SignalKind::PermissionRequest {
        action,
        resources,
        request,
        ..
    } = &signal.kind
    else {
        return None;
    };
    let mut facts = format!("{action}\n{request}").to_lowercase();

    for resource in resources {
        facts.push('\n');
        facts.push_str(&resource.requested.to_lowercase());
    }

    Some(facts)
}

/// A pattern naming a root (`/`, `~`, `$home`) matches only when the root
/// itself is the target, so `rm -rf /` does not veto `rm -rf /tmp/build`.
fn is_rooted(pattern: &str) -> bool {
    pattern.ends_with('/') || pattern.ends_with('~') || pattern.ends_with("$home")
}

/// Whether `pattern` occurs in `facts`; when `bounded`, only where it ends
/// the command or a word.
fn matches(facts: &str, pattern: &str, bounded: bool) -> bool {
    facts.match_indices(pattern).any(|(start, _)| {
        !bounded
            || facts
                .get(start.saturating_add(pattern.len())..)
                .and_then(|rest| rest.chars().next())
                .is_none_or(|next| {
                    next.is_whitespace() || matches!(next, '*' | '"' | '\'' | ';' | '&' | '|')
                })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::Resource;

    fn request(action: &str, resource: &str, reason: &str) -> Signal {
        Signal {
            agent_id: "a".into(),
            at: 1,
            kind: SignalKind::PermissionRequest {
                action: action.into(),
                resources: vec![Resource {
                    requested: resource.into(),
                    resolved: resource.into(),
                }],
                request: reason.into(),
                workspace: "/w".into(),
                user_requests: Vec::new(),
                host_decision: crate::effect::PermissionDecision::Allow,
            },
        }
    }

    #[test]
    fn built_in_and_project_patterns_veto() {
        let backstop = Backstop::new(vec!["git push --force origin main".into()]);

        assert_eq!(
            backstop.veto(&request("bash", "rm -rf / --no-preserve-root", "")),
            Some("rm -rf /")
        );
        assert!(
            backstop
                .veto(&request(
                    "write",
                    "key.pem",
                    "-----BEGIN RSA PRIVATE KEY-----"
                ))
                .is_some()
        );
        assert!(
            backstop
                .veto(&request("bash", "GIT PUSH --FORCE ORIGIN MAIN", ""))
                .is_some()
        );
        assert_eq!(
            backstop.veto(&request("bash", "rm -rf ./target", "clean build")),
            None
        );
        assert_eq!(
            backstop.veto(&request("bash", "rm -rf /tmp/build", "")),
            None
        );
        assert_eq!(
            backstop.veto(&request("bash", "rm -rf $HOME/project/dist", "")),
            None
        );
        assert!(backstop.veto(&request("bash", "rm -rf ~", "")).is_some());
        assert!(
            backstop
                .veto(&request("bash", "sudo rm -rf /* && echo", ""))
                .is_some()
        );
    }

    #[test]
    fn config_can_replace_defaults_and_learned_commands_still_apply() {
        let mut backstop = Backstop::from_config(BackstopConfig {
            replace: true,
            patterns: vec!["git push --force origin main".into()],
            confirm: Vec::new(),
        })
        .unwrap();

        assert!(backstop.veto(&request("bash", "rm -rf /", "")).is_none());
        assert!(
            backstop
                .veto(&request("bash", "git push --force origin main", ""))
                .is_some()
        );
        assert!(backstop.learn("DROP DATABASE production"));
        let restored = Backstop::from_config(BackstopConfig {
            replace: true,
            patterns: vec![],
            confirm: Vec::new(),
        })
        .unwrap()
        .with_learned(backstop.learned().to_vec());

        assert_eq!(
            restored.veto(&request("shell", "drop database production", "")),
            Some("drop database production")
        );
    }

    #[test]
    fn force_pushes_to_main_or_master_are_confirmed_and_feature_branches_are_not() {
        let backstop = Backstop::new(Vec::new());

        for command in [
            "git push --force origin main",
            "git push -f origin master",
            "git push --force-with-lease origin main",
            "git push origin main --force",
            "git push origin +master",
            "git fetch && git push -f origin main && echo done",
        ] {
            let signal = request("shell", command, "");

            assert!(backstop.confirm(&signal).is_some(), "{command}");
            assert!(backstop.veto(&signal).is_none(), "{command}");
        }

        for command in [
            "git push --force-with-lease origin feat/rebased",
            "git push -f origin main-fix",
            "git push origin main",
            "git push origin main --force-with-lease-if-includes",
        ] {
            assert!(
                backstop.confirm(&request("shell", command, "")).is_none(),
                "{command}"
            );
        }
    }
}
