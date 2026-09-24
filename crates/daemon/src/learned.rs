//! What the backstop and redactor learned, kept beside the engine's state in
//! files of their own so they can be reviewed and edited:
//! `learned-backstop.json` (`{"patterns": [...]}`) and
//! `learned-redaction.json` (`{"secret": [...], "safe": [...]}`).

use std::path::{Path, PathBuf};

use chauffeur_core::{Engine, LearnedShapes};
use serde::{Deserialize, Serialize};

const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Default, Deserialize, Serialize)]
struct LearnedBackstop {
    #[serde(default)]
    patterns: Vec<String>,
}

fn paths(state_file: &Path) -> (PathBuf, PathBuf) {
    let dir = state_file.parent().unwrap_or(Path::new("."));

    (
        dir.join("learned-backstop.json"),
        dir.join("learned-redaction.json"),
    )
}

/// The learned lists; a missing, oversized, or invalid file is empty.
pub fn load(state_file: &Path) -> (Vec<String>, LearnedShapes) {
    let (backstop, redaction) = paths(state_file);
    let backstop: LearnedBackstop = read(&backstop);

    (backstop.patterns, read(&redaction))
}

/// Write both lists atomically.
pub fn save(engine: &Engine, state_file: &Path) {
    let (backstop_path, redaction_path) = paths(state_file);
    let (patterns, shapes) = engine.learned();

    write(&backstop_path, &LearnedBackstop { patterns });
    write(&redaction_path, &shapes);
}

fn read<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    let readable = std::fs::metadata(path).is_ok_and(|metadata| metadata.len() <= MAX_BYTES);

    readable
        .then(|| std::fs::read(path).ok())
        .flatten()
        .and_then(|bytes| {
            serde_json::from_slice(&bytes)
                .inspect_err(|error| {
                    eprintln!(
                        "chauffeur: {} is not valid ({error}); ignored",
                        path.display()
                    )
                })
                .ok()
        })
        .unwrap_or_default()
}

fn write(path: &Path, value: &impl Serialize) {
    let temporary = path.with_extension("json.tmp");
    let result = serde_json::to_vec_pretty(value)
        .map_err(|error| error.to_string())
        .and_then(|bytes| std::fs::write(&temporary, bytes).map_err(|error| error.to_string()))
        .and_then(|()| std::fs::rename(&temporary, path).map_err(|error| error.to_string()));

    if let Err(error) = result {
        eprintln!(
            "chauffeur: learned list not saved to {}: {error}",
            path.display()
        );
    }
}
