//! The irreversible-harm backstop: the one deterministic gate outside System
//! One. It never routes; it vetoes a permission request whose facts match a
//! built-in or project pattern. Projects can add patterns, never remove them.

use std::path::Path;

use serde::Deserialize;

use crate::config::load_config;
use crate::signal::{Signal, SignalKind};

/// Case-insensitive substrings that mark an irreversible action.
const BUILT_IN: &[&str] = &[
    "rm -rf /",
    "rm -rf ~",
    "rm -rf $home",
    "rm -fr /",
    "mkfs",
    "of=/dev/",
    ":(){ :|:& };:",
    "chmod -r 777 /",
    "private key-----",
];
const MAX_PATTERNS: usize = 128;
const MAX_PATTERN_BYTES: usize = 256;

/// Strict JSON config: `{"patterns": ["git push --force origin main"]}`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BackstopConfig {
    #[serde(default)]
    pub patterns: Vec<String>,
}

pub struct Backstop {
    patterns: Vec<String>,
}

impl Backstop {
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::from_config(load_config(path)?)
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// Built-in patterns plus the config's, within bounds.
    pub fn from_config(config: BackstopConfig) -> Result<Self, String> {
        if config.patterns.len() > MAX_PATTERNS
            || config
                .patterns
                .iter()
                .any(|pattern| pattern.trim().is_empty() || pattern.len() > MAX_PATTERN_BYTES)
        {
            return Err(format!(
                "at most {MAX_PATTERNS} non-empty patterns of at most {MAX_PATTERN_BYTES} bytes"
            ));
        }

        Ok(Self::new(config.patterns))
    }

    /// Built-in patterns plus `extra`.
    #[must_use]
    pub fn new(extra: Vec<String>) -> Self {
        let patterns = BUILT_IN
            .iter()
            .map(|pattern| (*pattern).to_string())
            .chain(extra.into_iter().map(|pattern| pattern.to_lowercase()))
            .collect();

        Self { patterns }
    }

    /// The pattern a permission request matches, if any.
    #[must_use]
    pub fn veto(&self, signal: &Signal) -> Option<&str> {
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

        self.patterns
            .iter()
            .find(|pattern| matches(&facts, pattern))
            .map(String::as_str)
    }
}

/// A pattern naming a root (`/`, `~`, `$home`) matches only when the root
/// itself is the target, so `rm -rf /` does not veto `rm -rf /tmp/build`.
fn matches(facts: &str, pattern: &str) -> bool {
    let rooted = pattern.ends_with('/') || pattern.ends_with('~') || pattern.ends_with("$home");

    facts.match_indices(pattern).any(|(start, _)| {
        !rooted
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
}
