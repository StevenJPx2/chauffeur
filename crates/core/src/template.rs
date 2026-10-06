//! Wording kept in config: a question for System One, a message for the agent
//! or the user. Code fills named `{placeholders}`; the words are data.

use serde::{Deserialize, Serialize};

/// Bytes one template may hold.
pub const MAX_TEMPLATE_BYTES: usize = 4_096;

/// Text with named placeholders such as `{command}`. A placeholder is `{`,
/// one or more of `[a-z0-9_]`, then `}`; any other brace is literal text.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Template(String);

impl Template {
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The template, if it is 1 to [`MAX_TEMPLATE_BYTES`] bytes and names only
    /// `allowed` placeholders. `field` names it in the error.
    ///
    /// # Errors
    ///
    /// An empty or oversized template, or an unknown placeholder.
    pub fn check(&self, field: &str, allowed: &[&str]) -> Result<(), String> {
        if self.0.trim().is_empty() || self.0.len() > MAX_TEMPLATE_BYTES {
            return Err(format!("{field} must be 1-{MAX_TEMPLATE_BYTES} bytes"));
        }

        match placeholders(&self.0).find(|name| !allowed.contains(name)) {
            Some(unknown) => Err(format!(
                "{field} names {{{unknown}}}; it may use {}",
                if allowed.is_empty() {
                    "no placeholders".to_string()
                } else {
                    allowed
                        .iter()
                        .map(|name| format!("{{{name}}}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            )),
            None => Ok(()),
        }
    }

    /// The text with each placeholder replaced by its value, in one pass, so
    /// a value containing braces is never expanded again. A placeholder with
    /// no value is left as written.
    #[must_use]
    pub fn render(&self, values: &[(&str, &str)]) -> String {
        let mut out = String::with_capacity(self.0.len());
        let mut rest = self.0.as_str();

        while let Some(start) = rest.find('{') {
            out.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let name_len = after
                .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
                .unwrap_or(after.len());
            let value = (name_len > 0 && after[name_len..].starts_with('}'))
                .then(|| &after[..name_len])
                .and_then(|name| values.iter().find(|(key, _)| *key == name));

            match value {
                Some((_, value)) => {
                    out.push_str(value);
                    rest = &after[name_len + 1..];
                }
                None => {
                    out.push('{');
                    rest = after;
                }
            }
        }
        out.push_str(rest);

        out
    }
}

/// Every placeholder name in `text`, in order.
fn placeholders(text: &str) -> impl Iterator<Item = &str> {
    text.split('{').skip(1).filter_map(|part| {
        let end = part.find('}')?;
        let name = &part[..end];

        (!name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
        .then_some(name)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fills_named_placeholders_once_and_leaves_other_braces() {
        let template = Template::new("Run {command} in {dir}; JSON {\"a\": 1} and {Unknown}.");

        assert_eq!(
            template.render(&[("command", "echo {dir}"), ("dir", "/tmp")]),
            "Run echo {dir} in /tmp; JSON {\"a\": 1} and {Unknown}."
        );
    }

    #[test]
    fn checks_placeholders_and_size() {
        assert!(
            Template::new("Command: {command}")
                .check("q", &["command"])
                .is_ok()
        );
        assert!(
            Template::new("Command: {comand}")
                .check("q", &["command"])
                .unwrap_err()
                .contains("names {comand}; it may use {command}")
        );
        assert!(Template::new("  ").check("q", &[]).is_err());
        assert!(
            Template::new("x".repeat(MAX_TEMPLATE_BYTES + 1))
                .check("q", &[])
                .is_err()
        );
        assert!(Template::new("{\"json\": true}").check("q", &[]).is_ok());
    }
}
