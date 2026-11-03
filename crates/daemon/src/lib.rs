//! The Chauffeur daemon: loads models, redaction and rules, reloads them when
//! their files change, and answers JSON-RPC 2.0 lines on a Unix socket in a
//! folder only the user can open.
//!
//! | Method | Params | Result |
//! |---|---|---|
//! | `chauffeur.emit` | `event`, `deadline_ms` | `actions`: what each hooked rule returned |
//! | `chauffeur.events` | none | The last 256 runs, each step's answer or failure |
//! | `chauffeur.status` | none | Version, protocol, loaded rules, refused rule files |

mod paths;
mod route;
mod rpc;
mod socket;
mod state;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{PollWatcher, RecursiveMode, Watcher};

pub use paths::{Paths, private_folder};
pub use route::Router;
pub use socket::MAX_LINE;

/// How long file changes settle before a reload.
const SETTLE: Duration = Duration::from_millis(200);

/// How often configuration is checked when the platform cannot report
/// changes, such as in a sandbox that refuses `FSEvents`.
const POLL: Duration = Duration::from_secs(2);

/// Why the daemon stopped or would not start.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    /// Another daemon answers on the socket.
    #[error("a daemon is already running on {0}")]
    Running(PathBuf),
    /// The socket's folder is not private to the user.
    #[error("the socket folder {path} {reason}")]
    SocketFolder {
        /// The folder.
        path: PathBuf,
        /// What is wrong with it.
        reason: String,
    },
    /// The socket could not be bound.
    #[error("cannot bind {path}: {source}")]
    Socket {
        /// The socket.
        path: PathBuf,
        /// Why.
        source: std::io::Error,
    },
    /// Models or redaction could not load.
    #[error("{0}")]
    Config(String),
    /// Changes to configuration cannot be watched.
    #[error("cannot watch configuration: {0}")]
    Watch(#[from] notify::Error),
}

/// Loads everything for `paths`, binds the socket and serves until the
/// listener fails. Configuration that changes is reloaded; a change that
/// does not load keeps what was loaded before.
///
/// # Errors
///
/// A [`DaemonError`] if the daemon cannot start.
pub fn run(paths: &Paths) -> Result<(), DaemonError> {
    // Watch before the first load, so a change made while it loads, or just
    // after the socket opens, still reloads.
    let (send, receive) = mpsc::channel();
    let watcher = watch(&[&paths.distribution, &paths.config], &send)?;
    let router = Arc::new(Router::new(state::load(paths)?));
    let listener = socket::bind(&paths.socket)?;
    let (reloader, reload_paths) = (Arc::clone(&router), paths.clone());
    thread::Builder::new()
        .name("reload".into())
        .spawn(move || reload(&reloader, &reload_paths, &receive))
        .map_err(|source| DaemonError::Socket {
            path: paths.socket.clone(),
            source,
        })?;
    socket::serve(&listener, &router);
    drop(watcher);
    Ok(())
}

/// Watches `folders` with the platform's watcher, or by polling when that
/// watcher cannot start. Each change sends one message.
fn watch(folders: &[&Path], send: &mpsc::Sender<()>) -> Result<Box<dyn Watcher>, DaemonError> {
    let changed = |send: mpsc::Sender<()>| {
        move |event: notify::Result<notify::Event>| {
            if event.is_ok() {
                let _ = send.send(());
            }
        }
    };
    match start(notify::recommended_watcher(changed(send.clone())), folders) {
        Ok(watcher) => Ok(watcher),
        Err(error) => {
            eprintln!(
                "chauffeur: checking configuration for changes every {} s: {error}",
                POLL.as_secs()
            );
            let config = notify::Config::default().with_poll_interval(POLL);
            Ok(start(
                PollWatcher::new(changed(send.clone()), config),
                folders,
            )?)
        }
    }
}

fn start<W: Watcher + 'static>(
    watcher: notify::Result<W>,
    folders: &[&Path],
) -> notify::Result<Box<dyn Watcher>> {
    let mut watcher = watcher?;
    for folder in folders.iter().filter(|folder| folder.is_dir()) {
        watcher.watch(folder, RecursiveMode::Recursive)?;
    }
    Ok(Box::new(watcher))
}

fn reload(router: &Router, paths: &Paths, changes: &mpsc::Receiver<()>) {
    while changes.recv().is_ok() {
        while changes.recv_timeout(SETTLE).is_ok() {}
        match state::load(paths) {
            Ok(loaded) => router.replace(loaded),
            Err(error) => eprintln!("chauffeur: kept the loaded configuration: {error}"),
        }
    }
}
