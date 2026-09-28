//! Child processes that never outlive their owner: a guard kills the whole
//! process tree when dropped, and every wait has a deadline.

use std::process::{Child, Command};
use std::time::{Duration, Instant};

const POLL: Duration = Duration::from_millis(100);
/// Bounds the process-tree walk on kill.
const MAX_TREE: usize = 512;

/// How a waited-on child finished.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exit {
    /// Exited on its own; `None` when killed by a signal.
    Code(Option<i32>),
    /// Killed at the deadline.
    TimedOut,
}

/// Owns a child and kills its process tree on drop.
pub struct ChildGuard {
    child: Option<Child>,
    what: String,
}

impl ChildGuard {
    /// Spawn `command`; `what` names it in errors.
    ///
    /// # Errors
    /// The program cannot be started.
    pub fn spawn(command: &mut Command, what: &str) -> Result<Self, String> {
        let child = command
            .spawn()
            .map_err(|error| format!("start {what}: {error}"))?;

        Ok(Self {
            child: Some(child),
            what: what.to_string(),
        })
    }

    /// The exit code if the child has already exited.
    ///
    /// # Errors
    /// The child's status cannot be read.
    pub fn exited(&mut self) -> Result<Option<Exit>, String> {
        let Some(child) = self.child.as_mut() else {
            return Ok(Some(Exit::Code(None)));
        };

        child
            .try_wait()
            .map(|status| status.map(|status| Exit::Code(status.code())))
            .map_err(|error| format!("wait for {}: {error}", self.what))
    }

    /// Wait up to `timeout`, killing the tree if it has not exited by then.
    ///
    /// # Errors
    /// The child's status cannot be read.
    pub fn wait(&mut self, timeout: Duration) -> Result<Exit, String> {
        let deadline = Instant::now() + timeout;

        loop {
            if let Some(exit) = self.exited()? {
                self.child = None;

                return Ok(exit);
            }
            if Instant::now() >= deadline {
                self.kill();

                return Ok(Exit::TimedOut);
            }

            std::thread::sleep(POLL);
        }
    }

    /// Kill the child and all its descendants, then reap it.
    pub fn kill(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };

        let pids = descendants(child.id());
        let _ = child.kill();
        if !pids.is_empty() {
            let _ = Command::new("kill")
                .arg("-KILL")
                .args(pids.iter().map(u32::to_string))
                .stderr(std::process::Stdio::null())
                .status();
        }
        let _ = child.wait();
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Every live descendant of `root`, found with `pgrep -P` before anything is
/// killed so none is reparented away from the walk.
fn descendants(root: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let mut frontier = vec![root];

    while let Some(pid) = frontier.pop() {
        if found.len() >= MAX_TREE {
            break;
        }

        let children = Command::new("pgrep")
            .arg("-P")
            .arg(pid.to_string())
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
            .unwrap_or_default();
        for child in children
            .lines()
            .filter_map(|line| line.trim().parse::<u32>().ok())
        {
            found.push(child);
            frontier.push(child);
        }
    }

    found
}

/// Run `command` to completion within `timeout`.
///
/// # Errors
/// The program cannot be started or waited on.
pub fn run(command: &mut Command, what: &str, timeout: Duration) -> Result<Exit, String> {
    ChildGuard::spawn(command, what)?.wait(timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_out_and_kills_grandchildren() {
        let marker = std::env::temp_dir().join(format!("bench-kill-{}", std::process::id()));
        let _ = std::fs::remove_file(&marker);
        let script = format!("(sleep 1; touch {}) & wait", marker.display());
        let exit = run(
            Command::new("sh").arg("-c").arg(script),
            "sleeper",
            Duration::from_millis(300),
        )
        .unwrap();

        assert_eq!(exit, Exit::TimedOut);
        std::thread::sleep(Duration::from_millis(1200));
        assert!(!marker.exists(), "grandchild survived the kill");
    }

    #[test]
    fn reports_exit_codes() {
        let exit = run(
            Command::new("sh").arg("-c").arg("exit 3"),
            "sh",
            Duration::from_secs(5),
        )
        .unwrap();

        assert_eq!(exit, Exit::Code(Some(3)));
    }
}
