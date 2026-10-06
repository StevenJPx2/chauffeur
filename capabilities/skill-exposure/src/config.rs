//! Skill exposure's tunable bars and budget: shipped in
//! `skills/config/skill-exposure.json`, overlaid by your `skill-exposure.json`.

use std::path::Path;

use serde::Deserialize;

use chauffeur_core::{Confidence, Template, Threshold, load_layered};

use crate::MAX_OPTIONS;

/// The shipped config (`skills/config/skill-exposure.json`), compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/skill-exposure.json");

/// When a skill attaches, when one is handed over mid-turn, and how much one
/// signal may attach.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillExposureConfig {
    /// A skill the request needs attaches on a confident yes at this bar.
    pub needed: Threshold,
    /// A project's skill applies in its project unless Jev says no at this
    /// bar, confidently.
    pub in_project: Threshold,
    /// Mid-turn hand-overs.
    pub drift: Drift,
    /// What one signal may attach.
    pub budget: Budget,
    /// The wording of the questions to System One and of what is delivered.
    pub texts: SkillTexts,
}

/// The skill questions to System One and the messages delivered with a skill.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillTexts {
    /// Whether a skill serves what the agent asked for: `{need}`,
    /// `{user_request}`, `{skill}`, `{description}`.
    pub request: Template,
    /// Whether a skill of the session's own project applies: `{skill}`,
    /// `{description}`, `{workspace}`.
    pub project: Template,
    /// Whether the agent needs a skill the request names: `{word}` (the skill's
    /// leading word), `{skill}`, `{description}`.
    pub named: Template,
    /// Whether the request needs a skill nothing points at: `{skill}`,
    /// `{description}`, `{workspace}`.
    pub offered: Template,
    /// `{workspace}` when the session reports no directory.
    pub unknown_workspace: Template,
    /// Which skill, if any, improves the agent's latest tool action: `{action}`.
    pub drift: Template,
    /// `{action}` when no tool action is known.
    pub unknown_action: Template,
    /// The choice that hands over nothing.
    pub no_skill: Template,
    /// The message delivered with a mid-turn hand-over: `{skill}`.
    pub drift_notice: Template,
    /// Names an attached skill for the user: `{skill}`.
    pub label: Template,
}

impl SkillTexts {
    fn checked(self) -> Result<Self, String> {
        let skill = &["skill"][..];

        for (field, template, allowed) in [
            (
                "texts.request",
                &self.request,
                &["need", "user_request", "skill", "description"][..],
            ),
            (
                "texts.project",
                &self.project,
                &["skill", "description", "workspace"][..],
            ),
            (
                "texts.named",
                &self.named,
                &["word", "skill", "description"][..],
            ),
            (
                "texts.offered",
                &self.offered,
                &["skill", "description", "workspace"][..],
            ),
            ("texts.unknown_workspace", &self.unknown_workspace, &[][..]),
            ("texts.drift", &self.drift, &["action"][..]),
            ("texts.unknown_action", &self.unknown_action, &[][..]),
            ("texts.no_skill", &self.no_skill, &[][..]),
            ("texts.drift_notice", &self.drift_notice, skill),
            ("texts.label", &self.label, skill),
        ] {
            template.check(field, allowed)?;
        }

        Ok(self)
    }
}

/// Mid-turn hand-overs need stronger evidence than prompt admission.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Drift {
    /// A hand-over is picked with at least this confidence.
    pub confidence: Confidence,
    /// Tool results between drift checks are skipped for this long, per agent.
    pub cooldown_seconds: u64,
    /// Skills handed over only after the agent actually drove a browser.
    pub browser_only: Vec<String>,
}

/// Jev decides whether a skill helps; this only bounds what attaching costs.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Skill bytes one signal may attach.
    pub bytes: u64,
    /// Skills one signal attaches, at most, most likely first.
    pub skills: usize,
}

impl SkillExposureConfig {
    /// The shipped config overlaid by your `skill-exposure.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable or invalid, a bar is outside `[0, 1]`, or
    /// a count is out of bounds.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered::<Self>(SHIPPED, path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The config, if its counts are within bounds.
    ///
    /// # Errors
    ///
    /// When `budget.skills` is not in `1..=64`, `drift.browser_only` lists
    /// more than 64 skills, or a text is empty, oversized, or names an unknown
    /// placeholder.
    pub fn checked(mut self) -> Result<Self, String> {
        if !(1..=MAX_OPTIONS).contains(&self.budget.skills) {
            return Err(format!("budget.skills must be in 1..={MAX_OPTIONS}"));
        }

        if self.drift.browser_only.len() > MAX_OPTIONS {
            return Err(format!(
                "drift.browser_only lists more than {MAX_OPTIONS} skills"
            ));
        }

        self.texts = self.texts.checked()?;

        Ok(self)
    }
}

impl Default for SkillExposureConfig {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
            .expect("shipped skill-exposure config is valid")
    }
}

#[cfg(test)]
mod tests {
    use chauffeur_core::Rule;

    use super::*;

    fn yours(name: &str, json: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "chauffeur-skill-exposure-{}-{name}.json",
            std::process::id()
        ));

        std::fs::write(&path, json).unwrap();
        path
    }

    fn load(name: &str, json: &str) -> Result<SkillExposureConfig, String> {
        let path = yours(name, json);
        let config = SkillExposureConfig::load(&path);

        std::fs::remove_file(path).unwrap();
        config
    }

    #[test]
    fn the_shipped_config_holds_the_previous_values() {
        let config =
            SkillExposureConfig::load(Path::new("/nonexistent/skill-exposure.json")).unwrap();

        assert_eq!(config, SkillExposureConfig::default());
        assert_eq!(config.needed.yes(), Rule::yes(0.7, 0.4));
        assert_eq!(config.in_project.unless_no(), Rule::unless_no(0.3, 0.4));
        assert_eq!(config.drift.confidence.value(), 0.7);
        assert_eq!(config.drift.cooldown_seconds, 60);
        assert_eq!(config.drift.browser_only, ["browser-harness"]);
        assert_eq!(
            config.budget,
            Budget {
                bytes: 65_536,
                skills: 4
            }
        );
        assert!(SkillExposureConfig::default().checked().is_ok());
    }

    #[test]
    fn your_nested_field_keeps_the_rest() {
        let config = load("nested", r#"{ "drift": { "cooldown_seconds": 5 } }"#).unwrap();

        assert_eq!(config.drift.cooldown_seconds, 5);
        assert_eq!(config.drift.confidence.value(), 0.7);
        assert_eq!(config.drift.browser_only, ["browser-harness"]);
        assert_eq!(config.budget.skills, 4);
    }

    #[test]
    fn your_text_replaces_one_wording_and_an_unknown_placeholder_is_an_error() {
        let config = load("text", r#"{ "texts": { "no_skill": "None of these." } }"#).unwrap();
        let unknown = load("unknown", r#"{ "texts": { "label": "skill {skil}" } }"#).unwrap_err();

        assert_eq!(config.texts.no_skill.as_str(), "None of these.");
        assert_eq!(
            config.texts.label,
            SkillExposureConfig::default().texts.label
        );
        assert!(unknown.contains("texts.label names {skil}"));
    }

    #[test]
    fn an_unknown_field_an_out_of_range_bar_or_a_zero_budget_is_an_error() {
        let typo = load("typo", r#"{ "budjet": { "skills": 2 } }"#).unwrap_err();
        let range = load("range", r#"{ "in_project": { "at": 1.5 } }"#).unwrap_err();
        let zero = load("zero", r#"{ "budget": { "skills": 0 } }"#).unwrap_err();

        assert!(typo.contains("budjet"));
        assert!(range.contains("outside [0, 1]"));
        assert!(zero.contains("budget.skills"));
    }
}
