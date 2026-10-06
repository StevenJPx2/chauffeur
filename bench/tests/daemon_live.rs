//! Opt-in contract test against the real `chauffeur` binary (needs
//! `TYPESAFE_API_KEY`; asks no model): `cargo test -p chauffeur-bench --test
//! daemon_live -- --ignored`.

use chauffeur_bench::daemon::{self, Daemon, DaemonDirs};
use chauffeur_bench::which;

#[test]
#[ignore = "starts the real chauffeur daemon"]
fn real_daemon_starts_healthy_and_dies_with_its_guard() {
    let dir = std::env::temp_dir().join(format!("chauffeur-bench-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let bin = which::resolve("chauffeur").unwrap();

    let dirs = DaemonDirs {
        state: dir.join("state"),
        config: dir.join("config"),
        skills: chauffeur_bench::runner::skills_dir().unwrap(),
    };
    let daemon = Daemon::start(&bin, &dirs, "rules", &dir.join("daemon.log"))
        .unwrap_or_else(|error| panic!("{error}"));
    let url = daemon.url();
    let port: u16 = url.rsplit(':').next().unwrap().parse().unwrap();
    assert!(daemon::healthy(port));

    drop(daemon);
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!daemon::healthy(port), "daemon on {url} outlived its guard");
    std::fs::remove_dir_all(&dir).unwrap();
}
