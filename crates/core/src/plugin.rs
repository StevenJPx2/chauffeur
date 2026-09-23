//! Plugins contribute steering rules. Each plugin is its own crate; core
//! imports none of them — the daemon composes the registry, as SourceFed's
//! daemon composes `SOURCE_MAP` from the provider packages.

use crate::rule::Rule;

pub trait Plugin: Send + Sync {
    /// Stable id, also the prefix of every rule id this plugin owns (`github`).
    fn id(&self) -> &'static str;
    fn rules(&self) -> Vec<Rule>;
}

/// Flatten plugins into one validated rule set. Every rule id must be
/// prefixed `<plugin id>:` and unique across the registry.
pub fn compose(plugins: &[Box<dyn Plugin>]) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for plugin in plugins {
        let prefix = format!("{}:", plugin.id());

        for rule in plugin.rules() {
            if !rule.id.starts_with(&prefix) {
                return Err(format!("rule {} must be prefixed {prefix}", rule.id));
            }

            if !seen.insert(rule.id.clone()) {
                return Err(format!("duplicate rule id {}", rule.id));
            }

            rules.push(rule);
        }
    }

    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rule::Rule;

    struct Fake(&'static str, Vec<&'static str>);

    impl Plugin for Fake {
        fn id(&self) -> &'static str {
            self.0
        }

        fn rules(&self) -> Vec<Rule> {
            self.1
                .iter()
                .map(|id| Rule::new(id, id, "s", "r"))
                .collect()
        }
    }

    #[test]
    fn compose_enforces_prefix_and_uniqueness() {
        let ok: Vec<Box<dyn Plugin>> = vec![
            Box::new(Fake("a", vec!["a:x"])),
            Box::new(Fake("b", vec!["b:y"])),
        ];
        assert_eq!(compose(&ok).expect("valid").len(), 2);

        let bad_prefix: Vec<Box<dyn Plugin>> = vec![Box::new(Fake("a", vec!["b:x"]))];
        assert!(compose(&bad_prefix).is_err());

        let dup: Vec<Box<dyn Plugin>> = vec![Box::new(Fake("a", vec!["a:x", "a:x"]))];
        assert!(compose(&dup).is_err());
    }
}
