//! The audit log: every run, one JSON line each, kept across restarts.

use std::collections::VecDeque;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde_json::Value;

/// Past this size the log moves to `audit.1.jsonl`, replacing the one there,
/// so the two files together stay within twice this.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

const FILE: &str = "audit.jsonl";
const ROTATED: &str = "audit.1.jsonl";

/// How often [`follow`] looks for new lines.
const FOLLOW_EVERY: Duration = Duration::from_millis(250);

/// Appends runs to `audit.jsonl` in a folder only the user can open.
pub struct Audit {
    folder: PathBuf,
    max_bytes: u64,
    writing: Mutex<()>,
}

impl Audit {
    /// The log in `folder`, moving to `audit.1.jsonl` past `max_bytes`.
    #[must_use]
    pub fn new(folder: &Path, max_bytes: u64) -> Self {
        Self {
            folder: folder.to_owned(),
            max_bytes,
            writing: Mutex::new(()),
        }
    }

    /// Appends `entry`, with the time it was written as `at`: always to the
    /// microsecond, so every `at` has one width and they sort as text.
    ///
    /// # Errors
    ///
    /// The folder or the file cannot be created, moved or written.
    pub fn append(&self, mut entry: Value) -> io::Result<()> {
        if let Value::Object(fields) = &mut entry {
            fields.insert(
                "at".into(),
                Value::from(format!("{:.6}", jiff::Timestamp::now())),
            );
        }
        let mut line = entry.to_string();
        line.push('\n');
        let _writing = self
            .writing
            .lock()
            .map_err(|_| io::Error::other("a writer panicked"))?;
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.folder)?;
        let path = self.folder.join(FILE);
        let size = fs::metadata(&path).map_or(0, |metadata| metadata.len());
        let adding = u64::try_from(line.len()).unwrap_or(u64::MAX);
        if size > 0 && size.saturating_add(adding) > self.max_bytes {
            fs::rename(&path, self.folder.join(ROTATED))?;
        }
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(&path)?
            .write_all(line.as_bytes())
    }
}

/// The last `count` lines of the log in `folder`, oldest first. A missing
/// log has none.
///
/// # Errors
///
/// A log file exists but cannot be read.
pub fn recent(folder: &Path, count: usize) -> io::Result<Vec<String>> {
    let mut lines = VecDeque::with_capacity(count.min(1024));
    for name in [ROTATED, FILE] {
        let file = match File::open(folder.join(name)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for line in BufReader::new(file).lines() {
            lines.push_back(line?);
            if lines.len() > count {
                lines.pop_front();
            }
        }
    }
    Ok(lines.into())
}

/// Calls `each` with the last `count` lines of the log in `folder`, oldest
/// first, then with every line written from then on, until reading fails.
///
/// The recent lines are read through the file that is then followed, so no
/// line is written between them unseen. That file stays open, so when the
/// log moves to `audit.1.jsonl` its last lines are read before the new file
/// is read from its beginning.
///
/// # Errors
///
/// The log exists but cannot be read.
pub fn follow(folder: &Path, count: usize, mut each: impl FnMut(&str)) -> io::Result<()> {
    let path = folder.join(FILE);
    let mut following = open(&path)?;
    let mut partial = String::new();
    for line in start(folder, following.as_mut(), &mut partial, count)? {
        each(&line);
    }
    loop {
        thread::sleep(FOLLOW_EVERY);
        // Looked at first: every line of the old file was written before
        // the move, so reading it next finds them all.
        let moved = moved(&path, following.as_ref());
        if let Some(file) = following.as_mut() {
            read_lines(file, &mut partial, &mut each)?;
        }
        if moved {
            // Lines are written whole, so a part left over never ends.
            partial.clear();
            following = open(&path)?;
            if let Some(file) = following.as_mut() {
                read_lines(file, &mut partial, &mut each)?;
            }
        }
    }
}

/// The last `count` whole lines of the moved-aside log and of `following`,
/// read to its end; a last line not yet ended waits in `partial`.
fn start(
    folder: &Path,
    following: Option<&mut File>,
    partial: &mut String,
    count: usize,
) -> io::Result<VecDeque<String>> {
    let mut lines = VecDeque::with_capacity(count.min(1024));
    let mut keep = |line: &str| {
        lines.push_back(line.to_owned());
        if lines.len() > count {
            lines.pop_front();
        }
    };
    let rotated = open(&folder.join(ROTATED))?;
    // Moved aside since it was opened: its lines are the followed file's.
    if let Some(rotated) = rotated.filter(|rotated| !same(rotated, following.as_deref())) {
        for line in BufReader::new(rotated).lines() {
            keep(&line?);
        }
    }
    if let Some(file) = following {
        let mut reader = BufReader::new(file);
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line)? == 0 {
                break;
            }
            match line.strip_suffix('\n') {
                Some(whole) => keep(whole.trim_end()),
                None => *partial = line,
            }
        }
    }
    Ok(lines)
}

/// Whether `file` is the one being followed.
fn same(file: &File, following: Option<&File>) -> bool {
    let identity = |file: &File| file.metadata().ok().map(|data| (data.dev(), data.ino()));
    following
        .is_some_and(|following| identity(file).is_some() && identity(file) == identity(following))
}

/// The log at `path`, or none while it does not exist.
fn open(path: &Path) -> io::Result<Option<File>> {
    match File::open(path) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Whether `path` now names a file other than the one being followed.
fn moved(path: &Path, following: Option<&File>) -> bool {
    let identity = |metadata: fs::Metadata| (metadata.dev(), metadata.ino());
    let current = fs::metadata(path).ok().map(identity);
    let followed = following
        .and_then(|file| file.metadata().ok())
        .map(identity);
    current.is_some() && current != followed
}

/// Reads what was added to `file` and calls `each` with every line it
/// ends; a part with no newline yet waits in `partial`.
fn read_lines(
    file: &mut File,
    partial: &mut String,
    each: &mut impl FnMut(&str),
) -> io::Result<()> {
    file.read_to_string(partial)?;
    while let Some(end) = partial.find('\n') {
        let line: String = partial.drain(..=end).collect();
        each(line.trim_end());
    }
    Ok(())
}
