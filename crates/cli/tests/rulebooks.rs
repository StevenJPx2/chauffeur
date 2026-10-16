//! Rulebook validation uses the same strict rules parser as the daemon.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn validate(path: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_chauffeur"))
        .args(["skill", "validate"])
        .arg(path)
        .output()
        .unwrap()
}

#[test]
fn shipped_rules_and_rulebooks_validate_through_the_cli() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../skills");
    for folder in ["rules", "rulebooks"] {
        for file in std::fs::read_dir(root.join(folder)).unwrap() {
            let path = file.unwrap().path();
            if path
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                let output = validate(&path);
                assert!(
                    output.status.success(),
                    "{}: {}",
                    path.display(),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
    }
}

struct InvalidBook(PathBuf);

impl Drop for InvalidBook {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[test]
fn the_cli_rejects_a_review_that_exceeds_the_round_budget() {
    let mut book: serde_json::Value =
        serde_json::from_str(include_str!("../../../skills/rulebooks/goal.json")).unwrap();
    let mut second = book["rules"][0]["steps"][0].clone();
    second["id"] = serde_json::json!("extra");
    second.as_object_mut().unwrap().remove("review");
    book["rules"][0]["steps"]
        .as_array_mut()
        .unwrap()
        .push(second);
    let file = InvalidBook(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("invalid-review-{}.json", std::process::id())),
    );
    std::fs::write(&file.0, serde_json::to_vec(&book).unwrap()).unwrap();
    let output = validate(&file.0);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("2-round budget"));
}
