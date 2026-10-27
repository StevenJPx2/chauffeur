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

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

pub use paths::{Paths, private_folder};
pub use route::Router;
pub use socket::MAX_LINE;

/// How long file changes settle before a reload.
const SETTLE: Duration = Duration::from_millis(200);

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
    let router = Arc::new(Router::new(state::load(paths)?));
    let listener = socket::bind(&paths.socket)?;
    let (send, receive) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok() {
            let _ = send.send(());
        }
    })?;
    for folder in [&paths.distribution, &paths.config] {
        if folder.is_dir() {
            watcher.watch(folder, RecursiveMode::Recursive)?;
        }
    }
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

fn reload(router: &Router, paths: &Paths, changes: &mpsc::Receiver<()>) {
    while changes.recv().is_ok() {
        while changes.recv_timeout(SETTLE).is_ok() {}
        match state::load(paths) {
            Ok(loaded) => router.replace(loaded),
            Err(error) => eprintln!("chauffeur: kept the loaded configuration: {error}"),
        }
    }
}
