use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chauffeur_judges::Models;
use chauffeur_protocol::Rule;
use chauffeur_runtime::{Context, Redactor};

use crate::DaemonError;
use crate::paths::Paths;

/// The most rule files loaded from one source.
const MAX_RULES: usize = 512;

/// The most folders read for rules in one source.
const MAX_FOLDERS: usize = 256;

/// Everything loaded from the distribution and the user's configuration.
pub struct Loaded {
    /// Rules by ID; a user rule replaces a shipped one with the same ID.
    pub rules: Vec<Arc<Rule>>,
    /// Each rule file that was refused, with why.
    pub refused: Vec<String>,
    /// Models, redaction and keys.
    pub context: Arc<Context>,
    /// The rule each file holds, so a file that stops loading keeps it.
    files: BTreeMap<PathBuf, Arc<Rule>>,
}

/// Loads models, redaction and rules for `paths`.
///
/// A rule file that cannot load is refused on its own, with its path, and
/// the rest still load. When `previous` loaded it, that rule still runs, so
/// a file saved halfway through an edit changes nothing.
///
/// # Errors
///
/// Models or redaction that cannot load.
pub fn load(paths: &Paths, previous: Option<&Loaded>) -> Result<Loaded, DaemonError> {
    let mut model_files = json_files(&paths.distribution.join("models"));
    model_files.extend(existing(paths.config.join("models.json")));
    let models =
        Models::load(&model_files).map_err(|error| DaemonError::Config(error.to_string()))?;

    let mut redaction_files = vec![paths.distribution.join("safety").join("redaction.json")];
    redaction_files.extend(existing(paths.config.join("redaction.json")));
    let redactor =
        Redactor::load(&redaction_files).map_err(|error| DaemonError::Config(error.to_string()))?;

    let keys = Box::new(|name: &str| std::env::var(name).ok().filter(|key| !key.is_empty()));
    let mut loaded = Loaded {
        rules: Vec::new(),
        refused: Vec::new(),
        context: Arc::new(Context {
            models,
            redactor,
            keys,
        }),
        files: BTreeMap::new(),
    };
    for folder in [paths.distribution.join("rules"), paths.config.join("rules")] {
        for path in json_files_below(&folder) {
            let kept = previous.and_then(|previous| previous.files.get(&path));
            match (load_rule(&folder, &path), kept) {
                (Ok(rule), _) => loaded.add(path, Arc::new(rule)),
                (Err(reason), Some(kept)) => {
                    loaded.refused.push(format!(
                        "{}: {reason}; the version loaded before still runs",
                        path.display()
                    ));
                    loaded.add(path, Arc::clone(kept));
                }
                (Err(reason), None) => {
                    loaded.refused.push(format!("{}: {reason}", path.display()));
                }
            }
        }
    }
    Ok(loaded)
}

impl Loaded {
    /// Adds the rule `path` holds, in place of any with its ID.
    fn add(&mut self, path: PathBuf, rule: Arc<Rule>) {
        self.rules.retain(|loaded| loaded.id() != rule.id());
        self.rules.push(Arc::clone(&rule));
        self.files.insert(path, rule);
    }
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

/// The `.json` files in `folder` and below, sorted, at most [`MAX_RULES`]
/// from at most [`MAX_FOLDERS`] folders. A folder reached again through a
/// link is read once, by the path that reaches it first, so a link back to
/// a folder above it neither repeats its rules nor walks forever.
fn json_files_below(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut visited = BTreeSet::new();
    let mut pending = vec![folder.to_owned()];
    while let Some(next) = pending.pop() {
        if found.len() >= MAX_RULES || visited.len() >= MAX_FOLDERS {
            break;
        }
        let Ok(real) = std::fs::canonicalize(&next) else {
            continue;
        };
        if !visited.insert(real) {
            continue;
        }
        found.extend(json_files(&next));
        let mut subfolders: Vec<PathBuf> = std::fs::read_dir(&next)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        // Popped in name order, so the same folders always load the same IDs.
        subfolders.sort_by(|a, b| b.cmp(a));
        pending.extend(subfolders);
    }
    found.sort();
    found.truncate(MAX_RULES);
    found
}
