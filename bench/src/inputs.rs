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

/// Global OpenCode plugins a run disables unless its variant keeps them: they
/// change the prompt or read personal state.
pub const OPTIONAL_PLUGINS: &[&str] = &["ntfy-notify", "optmem", "sourcefed"];

/// One OpenCode setup under comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Variant {
    pub id: String,
    pub setup: Setup,
    /// Extra environment for OpenCode, such as `CHAUFFEUR_HOST_SKILLS=keep`.
    pub env: BTreeMap<String, String>,
    /// A folder of Chauffeur config overrides (`hosts/opencode.json`, …),
    /// relative to the suite root, copied into the run's config folder.
    pub config: Option<PathBuf>,
    /// Plugins from [`OPTIONAL_PLUGINS`] this variant keeps.
    pub plugins: Vec<String>,
}

impl Variant {
    /// Plain OpenCode with no extras, as `verify` lays out a repo.
    #[must_use]
    pub fn base(id: &str) -> Self {
        Self {
            id: id.into(),
            setup: Setup::Base,
            env: BTreeMap::new(),
            config: None,
            plugins: Vec::new(),
        }
    }

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
    config: Option<PathBuf>,
    #[serde(default)]
    plugins: Vec<String>,
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

    /// Stub MCP servers as `(name, spec)`: each `mcp/<name>.json`, sorted.
    ///
    /// # Errors
    /// An unreadable `mcp/` folder or a server name that is not a plain id.
    pub fn mcp_servers(&self) -> Result<Vec<(String, PathBuf)>, String> {
        let Some(dir) = existing_dir(self.dir.join("mcp")) else {
            return Ok(Vec::new());
        };
        let entries =
            std::fs::read_dir(&dir).map_err(|error| format!("read {}: {error}", dir.display()))?;
        let mut servers = Vec::new();

        for entry in entries {
            let path = entry
                .map_err(|error| format!("read {}: {error}", dir.display()))?
                .path();
            let Some(name) = path
                .file_stem()
                .filter(|_| path.extension().is_some_and(|ext| ext == "json"))
                .and_then(|stem| stem.to_str())
            else {
                continue;
            };

            check_id(name, "mcp server")?;
            servers.push((name.to_string(), path.clone()));
        }
        servers.sort();

        Ok(servers)
    }
}

fn existing_dir(path: PathBuf) -> Option<PathBuf> {
    path.is_dir().then_some(path)
}

/// Parse `variants.json` text. A `config` path stays relative to the suite
/// root until [`load_variants`] resolves it.
///
/// # Errors
/// Unknown fields, a Chauffeur variant without `disable`, a base variant
/// with a non-empty `disable` or a `config`, a `config` outside the suite, a
/// kept plugin that is not optional, or a missing, unsafe, or duplicate id.
pub fn parse_variants(text: &str) -> Result<Vec<Variant>, String> {
    let raw: Vec<RawVariant> =
        serde_json::from_str(text).map_err(|error| format!("parse variants: {error}"))?;
    let mut seen = BTreeSet::new();
    let mut variants = Vec::with_capacity(raw.len());

    for mut variant in raw {
        check_id(&variant.id, "variant")?;
        if !seen.insert(variant.id.clone()) {
            return Err(format!("variant {} is listed twice", variant.id));
        }

        let setup = match (variant.chauffeur, variant.disable.take()) {
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
        check_extras(&variant, &setup)?;
        variants.push(Variant {
            id: variant.id,
            setup,
            env: variant.env,
            config: variant.config,
            plugins: variant.plugins,
        });
    }

    Ok(variants)
}

/// A config overlay needs Chauffeur and must stay inside the suite; kept
/// plugins must be optional ones.
fn check_extras(variant: &RawVariant, setup: &Setup) -> Result<(), String> {
    if let Some(unknown) = variant
        .plugins
        .iter()
        .find(|plugin| !OPTIONAL_PLUGINS.contains(&plugin.as_str()))
    {
        return Err(format!(
            "variant {} keeps plugin {unknown}; optional plugins are {}",
            variant.id,
            OPTIONAL_PLUGINS.join(", ")
        ));
    }

    let Some(config) = &variant.config else {
        return Ok(());
    };

    if *setup == Setup::Base {
        return Err(format!(
            "variant {} has a config without chauffeur",
            variant.id
        ));
    }

    let inside = config
        .components()
        .all(|part| matches!(part, std::path::Component::Normal(_)));

    if inside && config.components().next().is_some() {
        Ok(())
    } else {
        Err(format!(
            "variant {} config {} must be a relative folder inside the suite",
            variant.id,
            config.display()
        ))
    }
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
    let mut variants =
        parse_variants(&text).map_err(|error| format!("{}: {error}", path.display()))?;

    for variant in &mut variants {
        if let Some(config) = &variant.config {
            let folder = root.join(config);
            if !folder.is_dir() {
                return Err(format!(
                    "variant {} config {} is not a folder",
                    variant.id,
                    folder.display()
                ));
            }
            variant.config = Some(folder);
        }
    }

    Ok(variants)
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

/// Read one task folder (`task.json` and `repo/`); its id is the folder name.
///
/// # Errors
/// An unreadable or invalid task, or one without `repo/`.
pub fn load_task(dir: &Path) -> Result<Task, String> {
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
