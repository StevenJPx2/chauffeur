//! Running a rule for an event, model calls aside: redacting its evidence,
//! then the whole run against a stand-in model. A run's worker thread
//! allocates outside the count, which covers the calling thread only.
#![expect(
    clippy::unwrap_used,
    reason = "a broken fixture stops the benchmark before it measures"
)]

use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use benchfmt::{CountingAlloc, Suite};
use chauffeur_judges::Models;
use chauffeur_protocol::{Event, Rule};
use chauffeur_runtime::{Context, Masking, Redactor, run};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc::new();

/// The most of a tool's output the `OpenCode` adapter sends.
const MAX_OUTPUT: usize = 64 * 1024;

const STAND_IN: &str = r#"{ "models": { "stand-in": { "format": "placeholder",
                                                     "otherwise": { "yes": 0.94, "no": 0.06 } } },
                            "tiers": { "system-one": ["stand-in"] } }"#;

fn repo(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

/// Build and test output, with paths and commit hashes but no credentials,
/// up to `bytes`.
fn build_output(bytes: usize) -> String {
    let lines = [
        "   Compiling chauffeur-core v1.0.0 (/Users/me/Projects/chauffeur/crates/core)\n",
        "test route::cooldown::a_rule_cools_down_per_session ... ok\n",
        "3f786850e387550fdab836ed7e6dc881de23001b fix(daemon): poll when FSEvents is refused\n",
    ]
    .concat();
    lines.repeat(bytes.checked_div(lines.len()).unwrap())
}

/// An environment dump with a known token and a credential-shaped value on
/// every few lines.
fn credentials() -> String {
    let lines = [
        "HOME=/Users/me\n",
        "GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123\n",
        "api_key=Zx9Qw3Er5Ty7Ui9Op1As3Df5Gh7Jk9Lz\n",
        "PATH=/opt/homebrew/bin:/usr/bin:/bin\n",
    ]
    .concat();
    lines.repeat(64)
}

/// The shipped redaction and a stand-in model that always answers yes.
fn context() -> Arc<Context> {
    let folder = std::env::temp_dir().join(format!("chauffeur-bench-run-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join("models.json"), STAND_IN).unwrap();
    let models = Models::load(&[folder.join("models.json")]).unwrap();
    std::fs::remove_dir_all(&folder).unwrap();
    Arc::new(Context {
        models,
        redactor: Redactor::load(&[repo("distribution/safety/redaction.json")]).unwrap(),
        keys: Box::new(|_| None),
    })
}

fn size(text: &str) -> u64 {
    u64::try_from(text.len()).unwrap()
}

fn main() {
    let context = context();
    let rule_text = std::fs::read_to_string(repo("distribution/rules/conventional-commits.json"));
    let rule = Arc::new(Rule::parse("conventional-commits", &rule_text.unwrap()).unwrap());
    let command = "git commit -m 'fixed stuff'";
    let output = build_output(MAX_OUTPUT);
    let secrets = credentials();
    let commit = serde_json::json!({
        "event": "tool.after", "session": "ses_1", "tool": "shell",
        "input": { "command": command }, "output": output
    });
    let commit = Event::parse(&commit.to_string()).unwrap();
    let redact = |text: &str| context.redactor.redact(text, &mut Masking::default());

    Suite::from_env("chauffeur-runtime")
        .with_alloc(&ALLOC)
        .bench("Redact/text=command", |b| {
            b.set_bytes(size(command));
            b.iter(|| redact(black_box(command)));
        })
        .bench("Redact/text=output-64KiB", |b| {
            b.set_bytes(size(&output));
            b.iter(|| redact(black_box(&output)));
        })
        .bench("Redact/text=credentials", |b| {
            b.set_bytes(size(&secrets));
            b.iter(|| redact(black_box(&secrets)));
        })
        .bench("Run/rule=conventional-commits", |b| {
            b.iter(|| run(&context, &rule, black_box(&commit), Duration::from_secs(2)));
        })
        .run();
}
