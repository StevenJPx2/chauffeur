//! Tool exposure's tunable tool sets and bars: shipped in
//! `skills/config/tool-exposure.json`, overlaid by your `tool-exposure.json`.

use std::path::Path;

use serde::Deserialize;

use chauffeur_core::{Confidence, Template, Threshold, load_layered};

/// The shipped config (`skills/config/tool-exposure.json`), compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/tool-exposure.json");
const MAX_BASE: usize = 64;
/// Tools named per group in a question, at most.
const MAX_LISTED_TOOLS: usize = 32;
/// Options in the missing-tool recovery choice, at most.
const MAX_RECOVERY_CANDIDATES: usize = 16;

/// Which tools are never judged, and the bars that hide and reveal the rest.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolExposureConfig {
    /// Tools never hidden: OpenCode's built-ins. Yours replaces the list.
    pub base: Vec<String>,
    /// Tools always hidden: Chauffeur owns skill loading, so the host's
    /// skill loader is never exposed.
    pub never_exposed: Vec<String>,
    /// At a context's first message, a group is hidden on a confident no at
    /// this bar.
    pub hide: Threshold,
    /// Later, or at the agent's request, a hidden group or Code Mode
    /// namespace is brought in on a confident yes at this bar.
    pub reveal: Threshold,
    /// A missing-tool recovery choice is taken with at least this confidence.
    pub pick_confidence: Confidence,
    /// Tools named per group in a question.
    pub listed_tools: usize,
    /// Hidden tools offered in a missing-tool recovery choice.
    pub recovery_candidates: usize,
    /// The wording of the questions to System One and of what is delivered.
    pub texts: ToolTexts,
}

/// The tool questions to System One, their options, and the Code Mode note.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolTexts {
    /// One tool in a group's listing: `{tool}`, `{description}`.
    pub listed_tool: Template,
    /// Ends a listing cut short: `{count}` tools not named.
    pub more_tools: Template,
    /// Whether a group is needed at a context's first message: `{group}`,
    /// `{tools}`.
    pub hide: Template,
    /// Whether a hidden group is needed now: `{group}`, `{tools}`.
    pub reveal: Template,
    /// Whether a Code Mode namespace is needed: `{namespace}`, `{size}`,
    /// `{examples}`.
    pub namespace: Template,
    /// What the agent asked for, opening each request question: `{need}`,
    /// `{user_request}`.
    pub request_asked: Template,
    /// Whether a hidden group serves the agent's request: `{asked}` (the
    /// rendered `request_asked`), `{group}`, `{tools}`.
    pub request_group: Template,
    /// Whether a namespace serves the agent's request: `{asked}`,
    /// `{namespace}`, `{size}`, `{examples}`.
    pub request_namespace: Template,
    /// Which hidden direct tool fits a missing-tool result: `{user_request}`,
    /// `{evidence}`.
    pub choose: Template,
    /// The recovery choice that reveals nothing.
    pub no_tool: Template,
    /// Whether the chosen tool is clearly needed: `{tool}`, `{description}`,
    /// `{user_request}`, `{evidence}`.
    pub confirm: Template,
    /// The confirmation option that reveals the tool.
    pub reveal_option: Template,
    /// The confirmation option that keeps it hidden as not needed.
    pub keep_option: Template,
    /// The confirmation option that keeps it hidden as uncertain.
    pub uncertain_option: Template,
    /// Names the Code Mode note for the user.
    pub code_mode_label: Template,
    /// One tool in the note: `{tool}`, `{description}`.
    pub code_mode_tool: Template,
    /// One namespace in the note: `{namespace}`, `{size}`, `{tools}` (the
    /// rendered `code_mode_tool` lines).
    pub code_mode_section: Template,
    /// The note delivered to the agent: `{sections}`.
    pub code_mode_note: Template,
}

impl ToolTexts {
    fn checked(self) -> Result<Self, String> {
        let group = &["group", "tools"][..];
        let namespace = &["namespace", "size", "examples"][..];
        let recovery = &["user_request", "evidence"][..];
        let tool = &["tool", "description"][..];

        for (field, template, allowed) in [
            ("texts.listed_tool", &self.listed_tool, tool),
            ("texts.more_tools", &self.more_tools, &["count"][..]),
            ("texts.hide", &self.hide, group),
            ("texts.reveal", &self.reveal, group),
            ("texts.namespace", &self.namespace, namespace),
            (
                "texts.request_asked",
                &self.request_asked,
                &["need", "user_request"][..],
            ),
            (
                "texts.request_group",
                &self.request_group,
                &["asked", "group", "tools"][..],
            ),
            (
                "texts.request_namespace",
                &self.request_namespace,
                &["asked", "namespace", "size", "examples"][..],
            ),
            ("texts.choose", &self.choose, recovery),
            ("texts.no_tool", &self.no_tool, &[][..]),
            (
                "texts.confirm",
                &self.confirm,
                &["tool", "description", "user_request", "evidence"][..],
            ),
            ("texts.reveal_option", &self.reveal_option, &[][..]),
            ("texts.keep_option", &self.keep_option, &[][..]),
            ("texts.uncertain_option", &self.uncertain_option, &[][..]),
            ("texts.code_mode_label", &self.code_mode_label, &[][..]),
            ("texts.code_mode_tool", &self.code_mode_tool, tool),
            (
                "texts.code_mode_section",
                &self.code_mode_section,
                &["namespace", "size", "tools"][..],
            ),
            (
                "texts.code_mode_note",
                &self.code_mode_note,
                &["sections"][..],
            ),
        ] {
            template.check(field, allowed)?;
        }

        Ok(self)
    }
}

impl ToolExposureConfig {
    /// The shipped config overlaid by your `tool-exposure.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable or invalid, a bar is outside `[0, 1]`, or
    /// a list or count is out of bounds.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered::<Self>(SHIPPED, path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The config, if its lists and counts are within bounds.
    ///
    /// # Errors
    ///
    /// When `base` or `never_exposed` holds more than 64 tools, a count is
    /// zero or above its bound, or a text is empty, oversized, or names an
    /// unknown placeholder.
    pub fn checked(mut self) -> Result<Self, String> {
        if self.base.len() > MAX_BASE {
            return Err(format!("more than {MAX_BASE} base tools"));
        }

        if self.never_exposed.len() > MAX_BASE {
            return Err(format!("more than {MAX_BASE} never-exposed tools"));
        }

        if !(1..=MAX_LISTED_TOOLS).contains(&self.listed_tools) {
            return Err(format!("listed_tools must be in 1..={MAX_LISTED_TOOLS}"));
        }

        if !(1..=MAX_RECOVERY_CANDIDATES).contains(&self.recovery_candidates) {
            return Err(format!(
                "recovery_candidates must be in 1..={MAX_RECOVERY_CANDIDATES}"
            ));
        }

        self.texts = self.texts.checked()?;

        Ok(self)
    }
}

impl Default for ToolExposureConfig {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
            .expect("shipped tool-exposure config is valid")
    }
}

#[cfg(test)]
mod tests {
    use chauffeur_core::Rule;

    use super::*;

    fn load(name: &str, json: &str) -> Result<ToolExposureConfig, String> {
        let path = std::env::temp_dir().join(format!(
            "chauffeur-tool-exposure-{}-{name}.json",
            std::process::id()
        ));

        std::fs::write(&path, json).unwrap();

        let config = ToolExposureConfig::load(&path);

        std::fs::remove_file(path).unwrap();
        config
    }

    #[test]
    fn the_shipped_config_holds_the_previous_values() {
        let config =
            ToolExposureConfig::load(Path::new("/nonexistent/tool-exposure.json")).unwrap();

        assert_eq!(config, ToolExposureConfig::default());
        assert_eq!(
            config.base,
            [
                "read",
                "edit",
                "patch",
                "write",
                "shell",
                "grep",
                "glob",
                "question",
                "subagent",
                "webfetch",
                "websearch",
            ]
        );
        assert_eq!(config.never_exposed, ["skill"]);
        assert_eq!(config.hide.no(), Rule::no(0.3, 0.4));
        assert_eq!(config.reveal.yes(), Rule::yes(0.7, 0.4));
        assert_eq!(config.pick_confidence.pick(), Rule::pick(0.4));
        assert_eq!(config.listed_tools, 6);
        assert_eq!(config.recovery_candidates, 4);
        assert!(ToolExposureConfig::default().checked().is_ok());
    }

    #[test]
    fn your_base_replaces_the_list_and_a_nested_field_keeps_the_rest() {
        let config = load("nested", r#"{ "base": ["read"], "reveal": { "at": 0.9 } }"#).unwrap();

        assert_eq!(config.base, ["read"]);
        assert_eq!(config.reveal.yes(), Rule::yes(0.9, 0.4));
        assert_eq!(config.hide.no(), Rule::no(0.3, 0.4));
        assert_eq!(config.never_exposed, ["skill"]);
    }

    #[test]
    fn your_text_replaces_one_wording_and_an_unknown_placeholder_is_an_error() {
        let config = load("text", r#"{ "texts": { "no_tool": "Nothing fits." } }"#).unwrap();
        let unknown = load("unknown", r#"{ "texts": { "hide": "Need {grup}?" } }"#).unwrap_err();

        assert_eq!(config.texts.no_tool.as_str(), "Nothing fits.");
        assert_eq!(config.texts.hide, ToolExposureConfig::default().texts.hide);
        assert!(unknown.contains("texts.hide names {grup}"));
    }

    #[test]
    fn an_unknown_field_an_out_of_range_bar_or_a_count_out_of_bounds_is_an_error() {
        let typo = load("typo", r#"{ "bsae": [] }"#).unwrap_err();
        let range = load("range", r#"{ "hide": { "confidence": -0.1 } }"#).unwrap_err();
        let zero = load("zero", r#"{ "listed_tools": 0 }"#).unwrap_err();
        let many = load("many", r#"{ "recovery_candidates": 17 }"#).unwrap_err();
        let base = load(
            "base",
            &format!(r#"{{ "base": {:?} }}"#, vec!["tool"; MAX_BASE + 1]),
        )
        .unwrap_err();

        assert!(typo.contains("bsae"));
        assert!(range.contains("outside [0, 1]"));
        assert!(zero.contains("listed_tools"));
        assert!(many.contains("recovery_candidates"));
        assert!(base.contains("base tools"));
    }
}
