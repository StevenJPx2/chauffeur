//! Notice when files change, and rebuild what was loaded from them.
//!
//! [`Watch`] fingerprints files and directories: each file's length and
//! modification time, and which files exist. It is checked on demand, so a
//! change applies at the caller's next natural boundary (the next signal)
//! rather than mid-operation, with no thread or OS event stream. Editors that
//! save by writing a temporary file and renaming it need no special handling.
//!
//! [`Reloading`] pairs a watch with a value and a hook that rebuilds the value
//! from the current one. A hook that fails keeps the current value, so a typo
//! never takes a capability away.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Checks closer together than this reuse the last answer.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);
/// Directories nested deeper than this are not walked.
const MAX_DEPTH: usize = 8;
/// A watch fingerprints at most this many files; the rest are ignored.
const MAX_FILES: usize = 4_096;

type Fingerprint = BTreeMap<PathBuf, (u64, Option<SystemTime>)>;

/// Detects changes under a set of files and directories.
///
/// A root path is followed through symlinks; inside a directory, symlinked
/// directories are not descended, so a link back into a watched tree cannot
/// loop. A missing path counts as empty, and its creation is a change.
pub struct Watch {
    roots: Vec<PathBuf>,
    last: Fingerprint,
    interval: Duration,
    checked: Option<Instant>,
}

impl Watch {
    /// Watch `roots`, taking their current state as the baseline.
    #[must_use]
    pub fn new(roots: Vec<PathBuf>) -> Self {
        let last = fingerprint(&roots);

        Self {
            roots,
            last,
            interval: DEFAULT_INTERVAL,
            checked: None,
        }
    }

    /// Check at most once per `interval`; zero checks every time.
    #[must_use]
    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.interval = interval;
        self
    }

    /// Whether anything under the roots changed since the last call that
    /// returned `true` (or since the watch began).
    pub fn changed(&mut self) -> bool {
        let now = Instant::now();

        if self
            .checked
            .is_some_and(|checked| now.duration_since(checked) < self.interval)
        {
            return false;
        }
        self.checked = Some(now);

        let current = fingerprint(&self.roots);
        if current == self.last {
            return false;
        }
        self.last = current;

        true
    }
}

fn fingerprint(roots: &[PathBuf]) -> Fingerprint {
    let mut files = Fingerprint::new();

    for root in roots {
        // A root is followed through a symlink, such as a linked skills folder.
        match std::fs::metadata(root) {
            Ok(metadata) if metadata.is_dir() => walk(root, 0, &mut files),
            Ok(metadata) => {
                files.insert(root.clone(), stamp(&metadata));
            }
            Err(_) => {}
        }
    }

    files
}

fn walk(directory: &Path, depth: usize, files: &mut Fingerprint) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };

    // The directory itself, so an emptied folder still differs.
    files.insert(directory.to_path_buf(), (0, None));

    for entry in entries.flatten() {
        if files.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };

        if kind.is_dir() {
            if depth < MAX_DEPTH {
                walk(&path, depth + 1, files);
            }
        } else if kind.is_symlink() {
            // A linked file counts by its target; a linked directory is skipped.
            match std::fs::metadata(&path) {
                Ok(metadata) if metadata.is_file() => {
                    files.insert(path, stamp(&metadata));
                }
                _ => {}
            }
        } else if let Ok(metadata) = entry.metadata() {
            files.insert(path, stamp(&metadata));
        }
    }
}

fn stamp(metadata: &std::fs::Metadata) -> (u64, Option<SystemTime>) {
    (metadata.len(), metadata.modified().ok())
}

/// The hook that rebuilds a value from the current one when its files change.
pub type Rebuild<T> = Box<dyn FnMut(&T) -> Result<T, String> + Send>;

/// A value rebuilt by a hook whenever the files it came from change.
pub struct Reloading<T> {
    value: T,
    watch: Watch,
    rebuild: Rebuild<T>,
    error: Option<String>,
    label: String,
}

impl<T> Reloading<T> {
    /// `value`, rebuilt by `rebuild` whenever `watch` sees a change. `label`
    /// names it in the log.
    pub fn new(label: impl Into<String>, value: T, watch: Watch, rebuild: Rebuild<T>) -> Self {
        Self {
            value,
            watch,
            rebuild,
            error: None,
            label: label.into(),
        }
    }

    /// The current value, rebuilt first if its files changed. A failed rebuild
    /// keeps the current value and logs the error once per distinct error.
    pub fn current(&mut self) -> &mut T {
        if self.watch.changed() {
            match (self.rebuild)(&self.value) {
                Ok(value) => {
                    self.value = value;
                    self.error = None;
                    eprintln!("chauffeur: reloaded {}", self.label);
                }
                Err(error) => {
                    if self.error.as_ref() != Some(&error) {
                        eprintln!(
                            "chauffeur: {} not reloaded, keeping the previous one: {error}",
                            self.label
                        );
                    }
                    self.error = Some(error);
                }
            }
        }

        &mut self.value
    }

    /// The value as it is, without checking for changes.
    pub fn peek(&mut self) -> &mut T {
        &mut self.value
    }

    /// Why the last rebuild failed, while the files are still broken.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("chauffeur-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn watch(dir: &Path) -> Watch {
        Watch::new(vec![dir.to_path_buf()]).with_interval(Duration::ZERO)
    }

    #[test]
    fn sees_files_added_edited_and_removed() {
        let dir = temp_dir("files");
        let mut watch = watch(&dir);

        assert!(!watch.changed());
        std::fs::write(dir.join("a.json"), "{}").unwrap();
        assert!(watch.changed());
        assert!(!watch.changed(), "a change is reported once");
        std::fs::write(dir.join("a.json"), "{\"x\": 1}").unwrap();
        assert!(watch.changed());
        std::fs::create_dir(dir.join("nested")).unwrap();
        std::fs::write(dir.join("nested/b.json"), "{}").unwrap();
        assert!(watch.changed());
        std::fs::remove_file(dir.join("a.json")).unwrap();
        assert!(watch.changed());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_root_changes_when_created() {
        let dir = temp_dir("missing");
        let file = dir.join("later.json");
        let mut watch = Watch::new(vec![file.clone()]).with_interval(Duration::ZERO);

        assert!(!watch.changed());
        std::fs::write(&file, "{}").unwrap();
        assert!(watch.changed());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checks_are_throttled() {
        let dir = temp_dir("throttle");
        let mut watch = Watch::new(vec![dir.clone()]).with_interval(Duration::from_secs(3600));

        assert!(!watch.changed());
        std::fs::write(dir.join("a.json"), "{}").unwrap();
        assert!(
            !watch.changed(),
            "within the interval the last answer stands"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failed_rebuild_keeps_the_current_value() {
        let dir = temp_dir("reload");
        let file = dir.join("n.txt");
        std::fs::write(&file, "1").unwrap();
        let read = file.clone();
        let mut value = Reloading::new(
            "n",
            1_u32,
            watch(&dir),
            Box::new(move |_| {
                std::fs::read_to_string(&read)
                    .map_err(|error| error.to_string())?
                    .trim()
                    .parse()
                    .map_err(|_| "not a number".to_string())
            }),
        );

        std::fs::write(&file, "2").unwrap();
        assert_eq!(*value.current(), 2);
        std::fs::write(&file, "oops").unwrap();
        assert_eq!(*value.current(), 2);
        assert_eq!(value.error(), Some("not a number"));
        std::fs::write(&file, "3").unwrap();
        assert_eq!(*value.current(), 3);
        assert_eq!(value.error(), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_hook_rebuilds_from_the_current_value() {
        let dir = temp_dir("carry");
        let mut count = Reloading::new("count", 10_u32, watch(&dir), Box::new(|old| Ok(old + 1)));

        std::fs::write(dir.join("touch"), "").unwrap();
        assert_eq!(*count.current(), 11);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
