//! What the daemon does with every event before any judge: reads it, and
//! checks whether a rule hooks it.
#![expect(
    clippy::unwrap_used,
    reason = "a broken fixture stops the benchmark before it measures"
)]

use std::hint::black_box;
use std::path::PathBuf;

use benchfmt::{CountingAlloc, Suite};
use chauffeur_protocol::{Event, Rule};

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc::new();

/// The most of a tool's output the `OpenCode` adapter sends.
const MAX_OUTPUT: usize = 64 * 1024;

fn shipped_rule() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../distribution/rules/conventional-commits.json");
    std::fs::read_to_string(path).unwrap()
}

/// A `tool.after` event for `command`, with test-run output up to `bytes`.
fn tool_after(command: &str, bytes: usize) -> String {
    let line = "test route::cooldown::a_rule_cools_down_per_session ... ok\n";
    let output: String = line.repeat(bytes.checked_div(line.len()).unwrap());
    serde_json::json!({
        "event": "tool.after", "session": "ses_1", "tool": "shell",
        "input": { "command": command }, "output": output
    })
    .to_string()
}

fn size(text: &str) -> u64 {
    u64::try_from(text.len()).unwrap()
}

fn main() {
    let rule_text = shipped_rule();
    let rule = Rule::parse("conventional-commits", &rule_text).unwrap();
    let commit = tool_after("git commit -m 'fixed stuff'", 0);
    let long = tool_after("cargo test --workspace", MAX_OUTPUT);
    let commit_event = Event::parse(&commit).unwrap();
    let listing = Event::parse(&tool_after("ls -la", 0)).unwrap();

    Suite::from_env("chauffeur-protocol")
        .with_alloc(&ALLOC)
        .bench("Parse/output=none", |b| {
            b.set_bytes(size(&commit));
            b.iter(|| Event::parse(black_box(&commit)));
        })
        .bench("Parse/output=64KiB", |b| {
            b.set_bytes(size(&long));
            b.iter(|| Event::parse(black_box(&long)));
        })
        .bench("Hooks/command=commit", |b| {
            b.iter(|| rule.hooks(black_box(&commit_event)));
        })
        .bench("Hooks/command=ls", |b| {
            b.iter(|| rule.hooks(black_box(&listing)));
        })
        .bench("RuleParse/rule=conventional-commits", |b| {
            b.iter(|| Rule::parse("conventional-commits", black_box(&rule_text)));
        })
        .run();
}
