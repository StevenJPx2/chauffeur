//! Bounded project-local JSON skill contracts, read from `.chauffeur/skills/`
//! in the workspace and its ancestors up to the Git root.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chauffeur_core::{Delivery, read_json_files};
use serde::Deserialize;

const MAX_DEPTH: usize = 8;
const MAX_FILES: usize = 16;
const MAX_SKILLS: usize = 32;
const MAX_FILE_BYTES: u64 = 16_384;
const MAX_STEPS: usize = 2;
const MAX_ID_BYTES: usize = 64;
const MAX_TEXT_BYTES: usize = 1_024;
const MAX_LABEL_BYTES: usize = 128;

/// One project-local follow-through: facts admit it, Jev confirms its steps
/// in order, and the effect is delivered once every step is confirmed.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    pub schema_version: u8,
    pub id: String,
    #[serde(rename = "match")]
    pub matching: Match,
    /// One or two judgments; the second is asked only after the first holds.
    pub steps: Vec<Step>,
    pub effect: FollowThrough,
    /// Deliver at most once per agent and workspace until the next user
    /// message.
    #[serde(default)]
    pub once: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Match {
    pub event: Event,
    #[serde(default)]
    pub tools_called_any: Vec<String>,
    #[serde(default)]
    pub tools_not_called: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Event {
    TurnEnd,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    pub question: String,
    /// Confirmed when P(yes) is at least this…
    pub yes_at_or_above: f32,
    /// …and the answer is at least this confident.
    pub minimum_confidence: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FollowThrough {
    pub label: String,
    pub delivery: Delivery,
    pub text: String,
}

impl Skill {
    fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err(format!(
                "unsupported schema_version {}",
                self.schema_version
            ));
        }
        if !identifier(&self.id) {
            return Err(format!(
                "id {:?} must be 1-{MAX_ID_BYTES} of [a-z0-9_-]",
                self.id
            ));
        }
        if !(1..=MAX_STEPS).contains(&self.steps.len()) {
            return Err(format!("{} must list 1-{MAX_STEPS} steps", self.id));
        }

        let mut ids = HashSet::new();

        for step in &self.steps {
            if !identifier(&step.id)
                || !ids.insert(&step.id)
                || !bounded_text(&step.question, MAX_TEXT_BYTES)
                || !probability(step.yes_at_or_above)
                || !probability(step.minimum_confidence)
            {
                return Err(format!("invalid judgment step {} in {}", step.id, self.id));
            }
        }

        let effect = &self.effect;

        if !bounded_text(&effect.text, MAX_TEXT_BYTES)
            || !bounded_text(&effect.label, MAX_LABEL_BYTES)
            || !matches!(effect.delivery, Delivery::Resume | Delivery::Wait)
        {
            return Err(format!("invalid follow-through in {}", self.id));
        }

        let matching = &self.matching;

        if let Some(tool) = matching
            .tools_called_any
            .iter()
            .chain(&matching.tools_not_called)
            .find(|tool| !identifier(tool))
        {
            return Err(format!("invalid tool {tool} in {}", self.id));
        }

        Ok(())
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'_'
        })
}

fn bounded_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= max_bytes
}

fn probability(value: f32) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

/// Every skill for `workspace`, from its Git root down to the workspace, so
/// a sibling repository never contributes. A relative workspace or one outside
/// a Git repository has none. IDs must be unique across all levels.
pub fn load(workspace: &str) -> Result<Vec<Skill>, String> {
    let path = Path::new(workspace);

    if !path.is_absolute() {
        return Ok(Vec::new());
    }

    let path =
        std::fs::canonicalize(path).map_err(|error| format!("resolve {workspace}: {error}"))?;
    let ancestors: Vec<PathBuf> = path
        .ancestors()
        .take(MAX_DEPTH)
        .map(Path::to_path_buf)
        .collect();
    let Some(root) = ancestors
        .iter()
        .position(|directory| directory.join(".git").exists())
    else {
        return Ok(Vec::new());
    };
    let mut skills = Vec::new();
    let mut ids = HashSet::new();

    for directory in ancestors.iter().take(root.saturating_add(1)).rev() {
        for (file, bytes) in read_skill_files(directory)? {
            let skill = parse(&bytes).map_err(|error| format!("{}: {error}", file.display()))?;

            if skills.len() == MAX_SKILLS {
                return Err(format!("{}: more than {MAX_SKILLS} skills", file.display()));
            }
            if !ids.insert(skill.id.clone()) {
                return Err(format!("{}: duplicate id {}", file.display(), skill.id));
            }

            skills.push(skill);
        }
    }

    Ok(skills)
}

/// JSON files in `directory/.chauffeur/skills`. Symlinked directories are
/// rejected so a contract cannot be read from outside the worktree;
/// [`read_json_files`] rejects symlinked files.
fn read_skill_files(directory: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let config = directory.join(".chauffeur");
    let folder = config.join("skills");

    for item in [&config, &folder] {
        if item
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(format!("{} must not be a symlink", item.display()));
        }
    }

    read_json_files(&folder, MAX_FILES, MAX_FILE_BYTES)
}

fn parse(bytes: &[u8]) -> Result<Skill, String> {
    let skill: Skill = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;

    skill.validate()?;

    Ok(skill)
}
