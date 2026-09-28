//! The benchmark's fixed inputs: `variants.json` and `tasks/<id>/task.json`,
//! parsed strictly and validated once here.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// How OpenCode is set up for a variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Setup {
    /// Plain OpenCode, with the Chauffeur plugin disabled.
    Base,
    /// OpenCode with Chauffeur, leaving out the named capabilities.
    Chauffeur { disable: Vec<String> },
}

/// One OpenCode setup under comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub id: String,
    pub setup: Setup,
    /// Extra environment for OpenCode, such as `CHAUFFEUR_HOST_SKILLS=keep`.
    pub env: BTreeMap<String, String>,
}

impl Variant {
    /// The `CHAUFFEUR_DISABLE` value for this variant; empty for base.
    #[must_use]
    pub fn disable_csv(&self) -> String {
        match &self.setup {
            Setup::Base => String::new(),
            Setup::Chauffeur { disable } => disable.join(","),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawVariant {
    id: String,
    chauffeur: bool,
    disable: Option<Vec<String>>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

/// A task's `task.json`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TaskSpec {
    pub prompt: String,
    pub check: Vec<String>,
    pub timeout_seconds: u64,
    pub tags: Vec<String>,
}

/// A task and the directory holding its `repo/`, `hidden/`, `bin/`, and
/// `solution/` folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub spec: TaskSpec,
    pub dir: PathBuf,
}

impl Task {
    /// The starting files.
    #[must_use]
    pub fn repo(&self) -> PathBuf {
        self.dir.join("repo")
    }

    /// Files copied over the agent's work before the check, if any.
    #[must_use]
    pub fn hidden(&self) -> Option<PathBuf> {
        existing_dir(self.dir.join("hidden"))
    }

    /// Stub commands prepended to `PATH`, if any.
    #[must_use]
    pub fn bin(&self) -> Option<PathBuf> {
        existing_dir(self.dir.join("bin"))
    }

    /// Files placed beside the repo, outside the project, if any. The prompt
    /// names the folder as `{outside}`; commands see it as `BENCH_OUTSIDE`.
    #[must_use]
    pub fn outside(&self) -> Option<PathBuf> {
        existing_dir(self.dir.join("outside"))
    }

    /// The prompt, with `{outside}` replaced by `folder`.
    #[must_use]
    pub fn prompt(&self, folder: &Path) -> String {
        self.spec
            .prompt
            .replace("{outside}", &folder.display().to_string())
    }

    /// The overlay that makes the check pass, if any.
    #[must_use]
    pub fn solution(&self) -> Option<PathBuf> {
        existing_dir(self.dir.join("solution"))
    }
}

fn existing_dir(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
}

/// Parse `variants.json` text.
///
/// # Errors
/// Unknown fields, a Chauffeur variant without `disable`, a base variant
/// with a non-empty `disable`, or a missing, unsafe, or duplicate id.
pub fn parse_variants(text: &str) -> Result<Vec<Variant>, String> {
    let raw: Vec<RawVariant> =
        serde_json::from_str(text).map_err(|error| format!("parse variants: {error}"))?;
    let mut seen = BTreeSet::new();
    let mut variants = Vec::with_capacity(raw.len());

    for variant in raw {
        check_id(&variant.id, "variant")?;
        if !seen.insert(variant.id.clone()) {
            return Err(format!("variant {} is listed twice", variant.id));
        }

        let setup = match (variant.chauffeur, variant.disable) {
            (true, Some(disable)) => Setup::Chauffeur { disable },
            (true, None) => {
                return Err(format!(
                    "variant {} enables chauffeur but has no disable list",
                    variant.id
                ));
            }
            (false, Some(disable)) if !disable.is_empty() => {
                return Err(format!(
                    "variant {} disables capabilities without chauffeur",
                    variant.id
                ));
            }
            (false, _) => Setup::Base,
        };
        variants.push(Variant {
            id: variant.id,
            setup,
            env: variant.env,
        });
    }

    Ok(variants)
}

/// Parse one `task.json`.
///
/// # Errors
/// Unknown or missing fields, an empty check, or a zero timeout.
pub fn parse_task_spec(text: &str) -> Result<TaskSpec, String> {
    let spec: TaskSpec = serde_json::from_str(text).map_err(|error| error.to_string())?;

    if spec.check.is_empty() {
        return Err("check is empty".into());
    }
    if spec.timeout_seconds == 0 {
        return Err("timeout_seconds is zero".into());
    }

    Ok(spec)
}

/// Read `<root>/variants.json`.
///
/// # Errors
/// An unreadable or invalid file.
pub fn load_variants(root: &Path) -> Result<Vec<Variant>, String> {
    let path = root.join("variants.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;

    parse_variants(&text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Read every `<root>/tasks/<id>/task.json`, sorted by id.
///
/// # Errors
/// An unreadable tasks folder, an invalid task, or a task without `repo/`.
pub fn load_tasks(root: &Path) -> Result<Vec<Task>, String> {
    let dir = root.join("tasks");
    let entries =
        std::fs::read_dir(&dir).map_err(|error| format!("read {}: {error}", dir.display()))?;
    let mut tasks = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| format!("read {}: {error}", dir.display()))?;
        let task_dir = entry.path();
        if !task_dir.join("task.json").is_file() {
            continue;
        }

        tasks.push(load_task(&task_dir)?);
    }
    tasks.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(tasks)
}

fn load_task(dir: &Path) -> Result<Task, String> {
    let id = dir
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("task folder {} is not UTF-8", dir.display()))?
        .to_string();
    check_id(&id, "task")?;
    let path = dir.join("task.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    let spec = parse_task_spec(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    let task = Task {
        id,
        spec,
        dir: dir.to_path_buf(),
    };

    if !task.repo().is_dir() {
        return Err(format!("task {} has no repo/ folder", task.id));
    }

    Ok(task)
}

/// Keep the items named in `wanted` (in their original order), or all of
/// them when `wanted` is `None`.
///
/// # Errors
/// A name that matches no item.
pub fn select<T: Clone>(
    items: &[T],
    wanted: Option<&[String]>,
    id: impl Fn(&T) -> &str,
    what: &str,
) -> Result<Vec<T>, String> {
    let Some(wanted) = wanted else {
        return Ok(items.to_vec());
    };

    if let Some(unknown) = wanted
        .iter()
        .find(|name| !items.iter().any(|item| id(item) == name.as_str()))
    {
        return Err(format!("unknown {what} {unknown}"));
    }

    Ok(items
        .iter()
        .filter(|item| wanted.iter().any(|name| name == id(item)))
        .cloned()
        .collect())
}

/// Ids become directory names, so they must be plain path segments.
fn check_id(id: &str, what: &str) -> Result<(), String> {
    let safe = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !id.starts_with('.');

    if safe {
        Ok(())
    } else {
        Err(format!(
            "{what} id {id:?} must be letters, digits, '-', '_', or '.'"
        ))
    }
}
