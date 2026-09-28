//! Runs the harness against a fixture suite with fake `opencode` and
//! `chauffeur` programs: no model, no daemon.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chauffeur_bench::inputs;
use chauffeur_bench::result::{self, RunResult};
use chauffeur_bench::runner::{self, RunOptions};
use chauffeur_bench::verify::{self, Outcome};

const FAKE_OPENCODE: &str = r#"#!/bin/sh
echo "agent $* | daemon=${CHAUFFEUR_DAEMON_URL:-none}" >> "$BENCH_LOG"
for last; do :; done
if [ "$last" = "sleep" ]; then sleep 30; fi
stub-tool from-agent
echo right > answer.txt
echo right > extra.txt
echo '{"type":"step_finish","part":{"cost":0.5,"tokens":{"input":100,"output":10,"reasoning":0,"cache":{"read":5,"write":0}}}}'
echo '{"type":"text","part":{"text":"done"}}'
"#;

const STUB: &str = "#!/bin/sh\necho \"stub-tool $*\" >> \"$BENCH_LOG\"\n";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn executable(path: &Path, text: &str) {
    write(path, text);
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A suite with a solvable task (`fix`) and one whose agent hangs (`slow`).
fn suite(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("chauffeur-bench-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    write(
        &root.join("variants.json"),
        r#"[{ "id": "base", "chauffeur": false }, { "id": "no-rules", "chauffeur": true, "disable": ["rules"] }]"#,
    );
    let fix = root.join("tasks/fix");
    write(
        &fix.join("task.json"),
        r#"{ "prompt": "fix it", "check": ["sh", "check.sh"], "timeout_seconds": 20, "tags": ["smoke"] }"#,
    );
    write(&fix.join("repo/answer.txt"), "wrong\n");
    write(
        &fix.join("hidden/check.sh"),
        "stub-tool from-check\ngrep -q right answer.txt && test -f extra.txt\n",
    );
    executable(&fix.join("bin/stub-tool"), STUB);
    write(&fix.join("solution/answer.txt"), "right\n");
    write(&fix.join("solution/.solve.sh"), "echo right > extra.txt\n");
    let slow = root.join("tasks/slow");
    write(
        &slow.join("task.json"),
        r#"{ "prompt": "sleep", "check": ["false"], "timeout_seconds": 1, "tags": [] }"#,
    );
    write(&slow.join("repo/README"), "slow\n");
    executable(&root.join("fake/opencode"), FAKE_OPENCODE);
    executable(
        &root.join("fake/chauffeur"),
        "#!/bin/sh\necho 'TYPESAFE_API_KEY is required' >&2\nexit 1\n",
    );

    root.canonicalize().unwrap()
}

fn options(root: &Path) -> RunOptions {
    RunOptions {
        tasks: None,
        variants: None,
        repeats: 1,
        parallel: 2,
        model: "test/model".into(),
        out: Some(root.join("out")),
        chauffeur: root.join("fake/chauffeur").display().to_string(),
        opencode: root.join("fake/opencode").display().to_string(),
    }
}

fn find<'a>(results: &'a [RunResult], variant: &str, task: &str) -> &'a RunResult {
    results
        .iter()
        .find(|result| result.variant == variant && result.task == task)
        .unwrap()
}

#[test]
fn verify_fails_unsolved_and_passes_solved() {
    let root = suite("verify");
    let tasks = inputs::load_tasks(&root).unwrap();

    let verdicts = verify::verify(&tasks, &root.join("scratch"));

    assert_eq!(verdicts[0].task, "fix");
    assert!(matches!(verdicts[0].unsolved, Outcome::Failed(_)));
    assert_eq!(verdicts[0].solved, Outcome::Passed);
    assert_eq!(verdicts[1].solved, Outcome::NoSolution);
    assert!(verdicts.iter().all(verify::Verdict::ok));
    assert!(verify::render(&verdicts).contains("| fix | fail (exit 1) | pass | yes |"));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn run_records_results_resumes_and_reports() {
    let root = suite("run");
    let out = root.join("out");

    runner::run(&root, &options(&root)).unwrap();

    let results = result::read_jsonl(&out.join("results.jsonl")).unwrap();
    assert_eq!(results.len(), 4);
    let pass = find(&results, "base", "fix");
    assert!(
        pass.passed && !pass.timed_out && pass.error.is_none(),
        "{pass:?}"
    );
    assert_eq!(pass.exit_code, Some(0));
    assert_eq!(pass.events.tokens.input, 100);
    assert_eq!(pass.events.last_text.as_deref(), Some("done"));
    assert!(pass.chauffeur.is_none());

    let slow = find(&results, "base", "slow");
    assert!(slow.timed_out && !slow.passed);
    assert!(
        slow.wall_seconds < 10.0,
        "timeout enforced: {}",
        slow.wall_seconds
    );

    let broken = find(&results, "no-rules", "fix");
    let error = broken.error.as_deref().unwrap();
    assert!(!broken.passed);
    assert!(error.contains("exited during startup"), "{error}");
    assert!(error.contains("TYPESAFE_API_KEY"), "{error}");

    let run_dir = out.join("runs/base/fix/1");
    let log = std::fs::read_to_string(run_dir.join("bench.log")).unwrap();
    assert!(
        log.contains("agent run --standalone --format json -m test/model fix it"),
        "{log}"
    );
    assert!(log.contains("| daemon=none"), "{log}");
    assert!(log.contains("stub-tool from-agent") && log.contains("stub-tool from-check"));
    let config = std::fs::read_to_string(run_dir.join("repo/.opencode/opencode.jsonc")).unwrap();
    assert!(config.contains("-chauffeur"));
    assert!(run_dir.join("repo/.git").is_dir());
    assert!(run_dir.join("result.json").is_file());
    let report = std::fs::read_to_string(out.join("report.md")).unwrap();
    assert!(report.contains("| Variants | base, no-rules |"), "{report}");
    assert!(report.contains("| fix | 1/1 | 0/0 (+1 err) |"), "{report}");
    assert!(
        report.contains("| no-rules | n/a | n/a | n/a | n/a |"),
        "{report}"
    );

    runner::run(&root, &options(&root)).unwrap();
    let again = std::fs::read_to_string(out.join("results.jsonl")).unwrap();
    assert_eq!(
        again.lines().count(),
        4,
        "finished jobs are skipped on resume"
    );
    std::fs::remove_dir_all(&root).unwrap();
}
