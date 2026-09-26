//! Skill exposure's tunable bars and budget: shipped in
//! `skills/config/skill-exposure.json`, overlaid by your `skill-exposure.json`.

use std::path::Path;

use serde::Deserialize;

use chauffeur_core::{Confidence, Threshold, load_layered};

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
    /// When `budget.skills` is not in `1..=64` or `drift.browser_only` lists
    /// more than 64 skills.
    pub fn checked(self) -> Result<Self, String> {
        if !(1..=MAX_OPTIONS).contains(&self.budget.skills) {
            return Err(format!("budget.skills must be in 1..={MAX_OPTIONS}"));
        }

        if self.drift.browser_only.len() > MAX_OPTIONS {
            return Err(format!(
                "drift.browser_only lists more than {MAX_OPTIONS} skills"
            ));
        }

        Ok(self)
    }
}

impl Default for SkillExposureConfig {
    fn default() -> Self {
        serde_json::from_str(SHIPPED).expect("shipped skill-exposure config is valid")
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
    fn an_unknown_field_an_out_of_range_bar_or_a_zero_budget_is_an_error() {
        let typo = load("typo", r#"{ "budjet": { "skills": 2 } }"#).unwrap_err();
        let range = load("range", r#"{ "in_project": { "at": 1.5 } }"#).unwrap_err();
        let zero = load("zero", r#"{ "budget": { "skills": 0 } }"#).unwrap_err();

        assert!(typo.contains("budjet"));
        assert!(range.contains("outside [0, 1]"));
        assert!(zero.contains("budget.skills"));
    }
}
