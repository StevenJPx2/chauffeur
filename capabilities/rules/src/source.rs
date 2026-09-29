//! Where rules come from: one folder of strict JSON files (`skills/rules/`
//! in this repo), and a project's `.chauffeur/rules/`, read from the
//! workspace and its ancestors up to the Git root.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use chauffeur_core::read_json_files;

use crate::rule::Rule;

const MAX_RULES: usize = 64;
const MAX_FILE_BYTES: u64 = 16_384;
const MAX_PROJECT_DEPTH: usize = 8;
const MAX_PROJECT_FILES: usize = 16;
const MAX_PROJECT_RULES: usize = 32;

/// Every rule in `directory`; a missing directory has none. IDs must be
/// unique.
pub fn load_dir(directory: &Path) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::new();

    collect(
        read_json_files(directory, MAX_RULES, MAX_FILE_BYTES)?,
        &mut rules,
        MAX_RULES,
    )?;

    Ok(rules)
}

/// Every project rule for `workspace`, from its Git root down to the
/// workspace, so a sibling repository never contributes. A relative
/// workspace or one outside a Git repository has none. IDs must be unique
/// across all levels.
pub fn load_project(workspace: &str) -> Result<Vec<Rule>, String> {
    let mut rules = Vec::new();

    for files in project_levels(workspace, "rules", MAX_FILE_BYTES)? {
        collect(files, &mut rules, MAX_PROJECT_RULES)?;
    }

    Ok(rules)
}

/// Every JSON file in `.chauffeur/<folder>/` from the workspace's Git root
/// down to the workspace, root first.
pub(crate) fn project_json(
    workspace: &str,
    folder: &str,
    max_bytes: u64,
) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    Ok(project_levels(workspace, folder, max_bytes)?
        .into_iter()
        .flatten()
        .collect())
}

/// One level's JSON files, as `(path, bytes)`.
type Files = Vec<(PathBuf, Vec<u8>)>;

/// `.chauffeur/<folder>/` files for each level from the Git root down. A
/// relative workspace or one outside a Git repository has none.
fn project_levels(workspace: &str, folder: &str, max_bytes: u64) -> Result<Vec<Files>, String> {
    let path = Path::new(workspace);

    if !path.is_absolute() {
        return Ok(Vec::new());
    }

    // A workspace that no longer exists, such as a removed worktree, has no
    // project files; that must not hide what is shipped.
    let path = match std::fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("resolve {workspace}: {error}")),
    };
    let ancestors: Vec<PathBuf> = path
        .ancestors()
        .take(MAX_PROJECT_DEPTH)
        .map(Path::to_path_buf)
        .collect();
    let Some(root) = ancestors
        .iter()
        .position(|directory| directory.join(".git").exists())
    else {
        return Ok(Vec::new());
    };

    ancestors
        .iter()
        .take(root.saturating_add(1))
        .rev()
        .map(|directory| project_files(directory, folder, max_bytes))
        .collect()
}

fn collect(
    files: Vec<(PathBuf, Vec<u8>)>,
    rules: &mut Vec<Rule>,
    limit: usize,
) -> Result<(), String> {
    let mut ids: HashSet<String> = rules.iter().map(|rule| rule.id.clone()).collect();

    for (file, bytes) in files {
        let rule =
            Rule::from_json(&bytes).map_err(|error| format!("{}: {error}", file.display()))?;

        if rules.len() == limit {
            return Err(format!("{}: more than {limit} rules", file.display()));
        }
        if !ids.insert(rule.id.clone()) {
            return Err(format!("{}: duplicate id {}", file.display(), rule.id));
        }

        rules.push(rule);
    }

    Ok(())
}

/// JSON files in `directory/.chauffeur/<folder>`. Symlinked directories are
/// rejected so a file cannot be read from outside the worktree;
/// [`read_json_files`] rejects symlinked files.
fn project_files(
    directory: &Path,
    folder: &str,
    max_bytes: u64,
) -> Result<Vec<(PathBuf, Vec<u8>)>, String> {
    let config = directory.join(".chauffeur");
    let folder = config.join(folder);

    for item in [&config, &folder] {
        if item
            .symlink_metadata()
            .is_ok_and(|meta| meta.file_type().is_symlink())
        {
            return Err(format!("{} must not be a symlink", item.display()));
        }
    }

    read_json_files(&folder, MAX_PROJECT_FILES, max_bytes)
}
