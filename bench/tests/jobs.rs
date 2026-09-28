use std::path::PathBuf;

use chauffeur_bench::inputs::{Setup, Task, TaskSpec, Variant};
use chauffeur_bench::jobs;

fn variant(id: &str) -> Variant {
    Variant {
        id: id.into(),
        setup: Setup::Base,
        env: Default::default(),
    }
}

fn task(id: &str) -> Task {
    Task {
        id: id.into(),
        spec: TaskSpec {
            prompt: "p".into(),
            check: vec!["true".into()],
            timeout_seconds: 1,
            tags: vec![],
        },
        dir: PathBuf::from("/nowhere").join(id),
    }
}

fn label(job: &jobs::Job<'_>) -> String {
    format!("{}/{}/{}", job.variant.id, job.task.id, job.repeat)
}

#[test]
fn plan_interleaves_variants_round_robin() {
    let variants = [variant("base"), variant("full")];
    let tasks = [task("a"), task("b")];

    let plan: Vec<String> = jobs::plan(&variants, &tasks, 2).iter().map(label).collect();

    assert_eq!(
        plan,
        vec![
            "base/a/1", "full/a/1", "base/b/1", "full/b/1", "base/a/2", "full/a/2", "base/b/2",
            "full/b/2",
        ]
    );
}

#[test]
fn pending_skips_jobs_with_a_result() {
    let out = std::env::temp_dir().join(format!("chauffeur-bench-jobs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    let variants = [variant("base"), variant("full")];
    let tasks = [task("a")];
    let plan = jobs::plan(&variants, &tasks, 1);
    let done = plan[0].dir(&out);
    std::fs::create_dir_all(&done).unwrap();
    std::fs::write(done.join("result.json"), "{}").unwrap();
    std::fs::create_dir_all(plan[1].dir(&out)).unwrap();

    let pending: Vec<String> = jobs::pending(&plan, &out).iter().map(label).collect();

    assert_eq!(done, out.join("runs/base/a/1"));
    assert_eq!(
        pending,
        vec!["full/a/1"],
        "a folder without result.json reruns"
    );
    std::fs::remove_dir_all(&out).unwrap();
}
