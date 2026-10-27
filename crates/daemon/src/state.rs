use std::path::{Path, PathBuf};
use std::sync::Arc;

use chauffeur_judges::Models;
use chauffeur_protocol::Rule;
use chauffeur_runtime::{Context, Redactor};

use crate::DaemonError;
use crate::paths::Paths;

/// The most rule files loaded from one source.
const MAX_RULES: usize = 512;

/// Everything loaded from the distribution and the user's configuration.
pub struct Loaded {
    /// Rules by ID; a user rule replaces a shipped one with the same ID.
    pub rules: Vec<Arc<Rule>>,
    /// Each rule file that was refused, with why.
    pub refused: Vec<String>,
    /// Models, redaction and keys.
    pub context: Arc<Context>,
}

/// Loads models, redaction and rules for `paths`.
///
/// # Errors
///
/// Models or redaction that cannot load. A rule file that cannot load is
/// refused on its own, with its path, and the rest still load.
pub fn load(paths: &Paths) -> Result<Loaded, DaemonError> {
    let mut model_files = json_files(&paths.distribution.join("models"));
    model_files.extend(existing(paths.config.join("models.json")));
    let models =
        Models::load(&model_files).map_err(|error| DaemonError::Config(error.to_string()))?;

    let mut redaction_files = vec![paths.distribution.join("safety").join("redaction.json")];
    redaction_files.extend(existing(paths.config.join("redaction.json")));
    let redactor =
        Redactor::load(&redaction_files).map_err(|error| DaemonError::Config(error.to_string()))?;

    let mut rules: Vec<Arc<Rule>> = Vec::new();
    let mut refused = Vec::new();
    for folder in [paths.distribution.join("rules"), paths.config.join("rules")] {
        for path in json_files_below(&folder) {
            match load_rule(&folder, &path) {
                Ok(rule) => {
                    rules.retain(|loaded| loaded.id() != rule.id());
                    rules.push(Arc::new(rule));
                }
                Err(reason) => refused.push(format!("{}: {reason}", path.display())),
            }
        }
    }
    let keys = Box::new(|name: &str| std::env::var(name).ok().filter(|key| !key.is_empty()));
    Ok(Loaded {
        rules,
        refused,
        context: Arc::new(Context {
            models,
            redactor,
            keys,
        }),
    })
}

fn load_rule(folder: &Path, path: &Path) -> Result<Rule, String> {
    let id = path.strip_prefix(folder).unwrap_or(path).with_extension("");
    let id = id.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    Rule::parse(&id, &text).map_err(|error| error.to_string())
}

fn existing(path: PathBuf) -> Option<PathBuf> {
    path.is_file().then_some(path)
}

/// The `.json` files directly in `folder`, sorted.
fn json_files(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    files
}

/// The `.json` files in `folder` and below, sorted, at most [`MAX_RULES`].
fn json_files_below(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_owned()];
    while let Some(next) = pending.pop() {
        if found.len() >= MAX_RULES {
            break;
        }
        found.extend(json_files(&next));
        let subfolders = std::fs::read_dir(&next)
            .into_iter()
            .flatten()
            .filter_map(Result::ok);
        pending.extend(
            subfolders
                .map(|entry| entry.path())
                .filter(|path| path.is_dir()),
        );
    }
    found.sort();
    found.truncate(MAX_RULES);
    found
}
