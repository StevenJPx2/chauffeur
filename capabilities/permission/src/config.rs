//! The wording permission adds around each contract's question, shipped in
//! `skills/config/permission.json` and overlaid by your `permission.json`.
//! The contracts in `skills/permission/*.json` keep their own question text.

use std::path::Path;

use chauffeur_core::{Template, load_layered};
use serde::Deserialize;

/// The shipped defaults, compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/permission.json");

/// Strict JSON config.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PermissionConfig {
    /// The wording of the question to System One.
    pub texts: PermissionTexts,
}

/// What permission says around a contract's own question.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PermissionTexts {
    /// A contract's question, led by the request itself: `{action}`,
    /// `{resources}` (comma separated), `{instructions}` (the contract's own).
    pub question: Template,
}

impl PermissionConfig {
    /// The shipped defaults overlaid by your `permission.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, or a text is empty or names a
    /// placeholder it may not use.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered::<Self>(SHIPPED, path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    fn checked(self) -> Result<Self, String> {
        self.texts
            .question
            .check("texts.question", &["action", "resources", "instructions"])?;

        Ok(self)
    }
}

impl Default for PermissionConfig {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
            .expect("shipped permission defaults are valid")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_file_is_the_default() {
        let config = PermissionConfig::load(Path::new("/nonexistent/permission.json")).unwrap();

        assert_eq!(config, PermissionConfig::default());
    }

    #[test]
    fn an_unknown_placeholder_is_an_error() {
        let path = std::env::temp_dir().join(format!(
            "chauffeur-permission-{}-typo.json",
            std::process::id()
        ));

        std::fs::write(&path, r#"{ "texts": { "question": "{acton}" } }"#).unwrap();

        let error = PermissionConfig::load(&path).unwrap_err();

        assert!(error.contains("names {acton}"), "{error}");
        std::fs::remove_file(path).unwrap();
    }
}
