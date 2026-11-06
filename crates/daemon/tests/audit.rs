//! The audit log through its files: appending, moving past its bound, and
//! reading the most recent runs back.
#![expect(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "the helpers fail the test on a broken fixture or a line missing a field"
)]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use chauffeur_daemon::audit::{Audit, MAX_BYTES, follow, recent};
use serde_json::{Value, json};

fn folder(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("chauffeur-audit-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    folder
}

fn run(index: u64) -> Value {
    json!({ "rule": "conventional-commits", "session": "s1", "elapsed_ms": index })
}

/// Each line's `elapsed_ms`, which these tests use as the run's number.
fn numbers(lines: &[String]) -> Vec<u64> {
    lines
        .iter()
        .map(|line| {
            serde_json::from_str::<Value>(line).unwrap()["elapsed_ms"]
                .as_u64()
                .unwrap()
        })
        .collect()
}

#[test]
fn runs_are_appended_with_their_time_in_a_folder_only_the_user_can_open() {
    let folder = folder("append");
    let audit = Audit::new(&folder, 1024 * 1024);

    audit.append(run(1)).unwrap();
    audit.append(run(2)).unwrap();

    let lines = recent(&folder, 10).unwrap();
    assert_eq!(numbers(&lines), [1, 2]);
    let first: Value = serde_json::from_str(&lines[0]).unwrap();
    let at = first["at"].as_str().unwrap();
    assert!(at.parse::<jiff::Timestamp>().is_ok(), "{first}");
    assert_eq!(
        at.len(),
        "2026-11-06T12:58:46.036546Z".len(),
        "to the microsecond, whatever the time: {at}"
    );
    assert_eq!(
        fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let file = fs::metadata(folder.join("audit.jsonl")).unwrap();
    assert_eq!(file.permissions().mode() & 0o777, 0o600);
    assert_eq!(numbers(&recent(&folder, 1).unwrap()), [2]);
}

#[test]
fn past_its_bound_the_log_moves_aside_once_and_reads_on_across_both_files() {
    let folder = folder("rotate");
    Audit::new(&folder, u64::MAX).append(run(1)).unwrap();
    let line = fs::metadata(folder.join("audit.jsonl")).unwrap().len();
    // Room for two lines and a half: every third line moves the file aside.
    let audit = Audit::new(&folder, line * 5 / 2);

    for index in 2..=5 {
        audit.append(run(index)).unwrap();
    }

    assert_eq!(
        numbers(&recent(&folder, 10).unwrap()),
        [3, 4, 5],
        "1 and 2 were replaced"
    );
    let moved = fs::read_to_string(folder.join("audit.1.jsonl")).unwrap();
    assert_eq!(
        numbers(&moved.lines().map(str::to_owned).collect::<Vec<_>>()),
        [3, 4]
    );
    assert_eq!(numbers(&recent(&folder, 2).unwrap()), [4, 5]);
}

#[test]
fn a_folder_with_no_log_has_no_runs() {
    assert_eq!(
        recent(&folder("missing"), 10).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn a_log_at_exactly_its_bound_stays_and_a_line_over_it_still_starts_one() {
    let folder = folder("bound");
    Audit::new(&folder, u64::MAX).append(run(1)).unwrap();
    let line = fs::metadata(folder.join("audit.jsonl")).unwrap().len();
    fs::remove_file(folder.join("audit.jsonl")).unwrap();

    let exact = Audit::new(&folder, line * 2);
    exact.append(run(1)).unwrap();
    exact.append(run(2)).unwrap();
    assert!(
        !folder.join("audit.1.jsonl").exists(),
        "two lines fill the bound exactly"
    );

    let tiny = folder.join("tiny");
    Audit::new(&tiny, 1).append(run(3)).unwrap();
    assert_eq!(
        numbers(&recent(&tiny, 10).unwrap()),
        [3],
        "a first line over the bound"
    );
}

#[test]
fn the_shipped_bound_holds_eight_mebibytes() {
    let folder = folder("shipped");
    let audit = Audit::new(&folder, MAX_BYTES);
    // 128 KiB entries: 63 make just under 8 MiB, and the 64th goes over.
    let padding = "x".repeat(128 * 1024);
    for _ in 0..63 {
        audit.append(json!({ "padding": padding })).unwrap();
    }
    assert!(
        !folder.join("audit.1.jsonl").exists(),
        "63 entries still fit"
    );
    audit.append(json!({ "padding": padding })).unwrap();
    assert!(
        folder.join("audit.1.jsonl").exists(),
        "the 64th moves the log aside"
    );
}

#[test]
fn a_log_that_cannot_be_read_is_an_error() {
    let folder = folder("unreadable");
    Audit::new(&folder, u64::MAX).append(run(1)).unwrap();
    fs::set_permissions(
        folder.join("audit.jsonl"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();

    assert!(recent(&folder, 10).is_err());
}

/// Follows the log in `folder` on its own thread, once it has started.
fn followed(folder: &std::path::Path) -> mpsc::Receiver<String> {
    let (send, received) = mpsc::channel();
    let watched = folder.to_owned();
    thread::spawn(move || follow(&watched, 0, |line| drop(send.send(line.to_owned()))));
    thread::sleep(Duration::from_millis(400));
    received
}

/// Each line's number, after checking every line is whole.
fn whole(seen: &[String]) -> Vec<u64> {
    for line in seen {
        assert!(
            serde_json::from_str::<Value>(line).is_ok(),
            "not whole: {line}"
        );
    }
    numbers(seen)
}

#[test]
fn following_notices_a_move_aside_when_the_new_log_is_already_longer() {
    let folder = folder("follow-longer");
    Audit::new(&folder, u64::MAX).append(run(0)).unwrap();
    let line = fs::metadata(folder.join("audit.jsonl")).unwrap().len();
    // Room for one line, so the next moves the log aside.
    let audit = Audit::new(&folder, line);
    let received = followed(&folder);

    let mut long = run(1);
    long["padding"] = json!("x".repeat(256));
    audit.append(long).unwrap();
    thread::sleep(Duration::from_millis(400));

    assert_eq!(whole(&received.try_iter().collect::<Vec<_>>()), [1]);
}

#[test]
fn following_keeps_lines_written_just_before_a_move_aside() {
    let folder = folder("follow-just-before");
    Audit::new(&folder, u64::MAX).append(run(0)).unwrap();
    let line = fs::metadata(folder.join("audit.jsonl")).unwrap().len();
    // Room for two lines, whatever digits their times have: the third
    // moves the log aside.
    let audit = Audit::new(&folder, line * 2 + 10);
    let received = followed(&folder);

    // Both within one of the follower's looks.
    audit.append(run(1)).unwrap();
    audit.append(run(2)).unwrap();
    thread::sleep(Duration::from_millis(400));

    assert!(folder.join("audit.1.jsonl").exists());
    assert_eq!(whole(&received.try_iter().collect::<Vec<_>>()), [1, 2]);
}

#[test]
fn following_a_log_that_cannot_be_read_is_an_error() {
    let folder = folder("follow-unreadable");
    Audit::new(&folder, u64::MAX).append(run(1)).unwrap();
    fs::set_permissions(
        folder.join("audit.jsonl"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    let (send, ended) = mpsc::channel();

    thread::spawn(move || {
        let _ = send.send(follow(&folder, 0, |_| {}).is_err());
    });

    assert_eq!(ended.recv_timeout(Duration::from_secs(2)), Ok(true));
}

#[test]
fn following_reads_each_new_line_once_across_a_move_aside() {
    let folder = folder("follow");
    Audit::new(&folder, u64::MAX).append(run(0)).unwrap();
    let line = fs::metadata(folder.join("audit.jsonl")).unwrap().len();
    let audit = Audit::new(&folder, line * 2);
    let (send, received) = mpsc::channel();
    let watched = folder;
    thread::spawn(move || follow(&watched, 0, |line| drop(send.send(line.to_owned()))));
    thread::sleep(Duration::from_millis(400));

    let mut seen = Vec::new();
    for index in 1..=4 {
        audit.append(run(index)).unwrap();
        // Slower than the follower's 250 ms, so it sees each move aside.
        thread::sleep(Duration::from_millis(400));
        while let Ok(line) = received.try_recv() {
            seen.push(line);
        }
    }

    assert_eq!(
        numbers(&seen),
        [1, 2, 3, 4],
        "run 0 was written before it followed"
    );
}

/// Each line `follow` calls back with, from the start, until `quiet` passes
/// with none.
fn collected(received: &mpsc::Receiver<String>, quiet: Duration) -> Vec<String> {
    let mut seen = Vec::new();
    while let Ok(line) = received.recv_timeout(quiet) {
        seen.push(line);
    }
    seen
}

#[test]
fn following_starts_with_the_recent_runs_and_misses_none_written_meanwhile() {
    let folder = folder("follow-recent");
    let audit = Audit::new(&folder, u64::MAX);
    for index in 1..=3 {
        audit.append(run(index)).unwrap();
    }
    let (send, received) = mpsc::channel();
    let watched = folder;

    thread::spawn(move || follow(&watched, 2, |line| drop(send.send(line.to_owned()))));
    // Written while it starts: before it reads the log, or after.
    for index in 4..=40 {
        audit.append(run(index)).unwrap();
    }

    let seen = whole(&collected(&received, Duration::from_millis(600)));
    let first = *seen.first().unwrap();
    assert!((2..=39).contains(&first), "{seen:?}");
    assert_eq!(
        seen,
        (first..=40).collect::<Vec<_>>(),
        "each once, in order"
    );
}

#[test]
fn a_line_still_being_written_when_following_starts_comes_once_whole() {
    let folder = folder("follow-partial");
    Audit::new(&folder, u64::MAX).append(run(1)).unwrap();
    let mut file = OpenOptions::new()
        .append(true)
        .open(folder.join("audit.jsonl"))
        .unwrap();
    write!(file, r#"{{ "elapsed_ms": 2"#).unwrap();
    let (send, received) = mpsc::channel();
    let watched = folder;

    thread::spawn(move || follow(&watched, 5, |line| drop(send.send(line.to_owned()))));
    thread::sleep(Duration::from_millis(400));
    writeln!(file, " }}").unwrap();

    assert_eq!(
        whole(&collected(&received, Duration::from_millis(600))),
        [1, 2]
    );
}
