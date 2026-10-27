#![expect(
    clippy::unwrap_used,
    reason = "the file helpers fail the test on a broken fixture"
)]

use std::fs;
use std::path::{Path, PathBuf};

use chauffeur_core::{Failure, Outcome, Panel, Plan, Step};
use chauffeur_judges::{
    Answer, Confidence, ConfigError, Evidence, Fallback, Milli, Models, NamedJudge, Question,
    Settled, Tier,
};

/// A fresh folder for one test's files.
fn folder(name: &str) -> PathBuf {
    let folder =
        std::env::temp_dir().join(format!("chauffeur-judges-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&folder);
    fs::create_dir_all(&folder).unwrap();
    folder
}

fn write(folder: &Path, name: &str, text: &str) -> PathBuf {
    let path = folder.join(name);
    fs::write(&path, text).unwrap();
    path
}

const SHIPPED: &str = r#"{
  "models": { "jev": { "format": "systemone", "endpoint": "https://api.typesafe.ai", "model": "jev-latest", "key": { "env": "TYPESAFE_API_KEY" } } },
  "tiers": { "system-one": ["jev"] }
}"#;

const ANSWERS: &str = r#"{ "answers": [
  { "evidence": { "commit": "fixed stuff" }, "p": { "yes": 0.94, "no": 0.06 } },
  { "evidence": { "commit": "fixed stuff" }, "p": { "yes": 0.01, "no": 0.99 } },
  { "evidence": { "commit": "deploy is on fire" }, "failed": "timeout" }
] }"#;

fn question() -> Question {
    Question::noul(
        "Does this commit message break Conventional Commits?",
        ["commit"],
    )
    .unwrap()
}

fn outcome(models: &Models, preferred: Option<&str>, commit: &str) -> Outcome<Answer> {
    let fallback = Fallback::build(models, Tier::SystemOne, preferred, |_| None).unwrap();
    let confidence = Confidence::Bar(Milli::new(700).unwrap());
    let evidence = Evidence::new([("commit", commit)]);
    let judge = NamedJudge::new(&fallback, &confidence, &evidence);
    let plan = Plan::new([Step::new("nudge", "quick", question())]).unwrap();
    let run = plan.run(&Panel::new().with("quick", &judge)).unwrap();
    run.verdict.get("nudge").unwrap().clone()
}

#[test]
fn a_user_placeholder_stands_in_for_jev_and_is_marked() {
    let folder = folder("placeholder");
    write(&folder, "answers.json", ANSWERS);
    let user = write(
        &folder,
        "models.json",
        r#"{ "models": { "stand-in": { "format": "placeholder", "answers": "answers.json" } },
             "tiers": { "system-one": ["jev", "stand-in"] } }"#,
    );
    let shipped = write(&folder, "jev.json", SHIPPED);
    let models = Models::load(&[shipped, user]).unwrap();

    let Outcome::Value(answer) = outcome(&models, None, "fixed stuff") else {
        panic!("no answer")
    };

    assert_eq!(
        answer.outcome(),
        &Settled::Outcome("yes".into()),
        "the first matching answer wins"
    );
    assert_eq!(
        answer.by().model,
        "stand-in",
        "jev has no key, so the tier falls back"
    );
    assert!(answer.by().placeholder);
    assert_eq!(
        outcome(&models, None, "deploy is on fire"),
        Outcome::Failed(Failure::Timeout)
    );
    assert_eq!(
        outcome(&models, None, "feat: add x"),
        Outcome::Failed(Failure::Unavailable)
    );
}

#[test]
fn the_next_model_answers_only_what_the_one_before_could_not() {
    let folder = folder("fallback");
    write(&folder, "answers.json", ANSWERS);
    let models = write(
        &folder,
        "models.json",
        r#"{ "models": {
               "first": { "format": "placeholder", "answers": "answers.json" },
               "second": { "format": "placeholder", "otherwise": { "yes": 0.2, "no": 0.8 } },
               "broken": { "format": "placeholder", "otherwise": { "maybe": 1 } }
             },
             "tiers": { "system-one": ["first", "second"] } }"#,
    );
    let models = Models::load(&[models]).unwrap();

    let Outcome::Value(timed_out) = outcome(&models, None, "deploy is on fire") else {
        panic!("no answer")
    };
    assert_eq!(
        (timed_out.by().model.as_str(), timed_out.outcome()),
        ("second", &Settled::Outcome("no".into()))
    );

    let Outcome::Value(preferred) = outcome(&models, Some("second"), "fixed stuff") else {
        panic!("no answer")
    };
    assert_eq!(
        preferred.by().model,
        "second",
        "a named model is asked first"
    );

    assert_eq!(
        outcome(&models, Some("broken"), "fixed stuff"),
        Outcome::Failed(Failure::Invalid),
        "an invalid answer never falls back"
    );
}

#[test]
fn a_tier_naming_an_unregistered_model_is_refused() {
    let folder = folder("unknown");
    let path = write(
        &folder,
        "models.json",
        r#"{ "tiers": { "system-one": ["missing"] } }"#,
    );

    let error = Models::load(&[path]).unwrap_err();

    assert!(
        matches!(&error, ConfigError::UnknownModel { tier: "system-one", model } if model == "missing")
    );
    assert_eq!(
        error.to_string(),
        "the system-one tier names `missing`, which no models.json registers"
    );
}

#[test]
fn a_malformed_recorded_answer_is_refused_with_its_position() {
    let folder = folder("malformed");
    write(
        &folder,
        "answers.json",
        r#"{ "answers": [ { "evidence": {}, "failed": "sleepy" } ] }"#,
    );
    let path = write(
        &folder,
        "models.json",
        r#"{ "models": { "p": { "format": "placeholder", "answers": "answers.json" } }, "tiers": { "system-one": ["p"] } }"#,
    );
    let models = Models::load(&[path]).unwrap();

    let error = Fallback::build(&models, Tier::SystemOne, None, |_| None)
        .err()
        .unwrap();

    assert!(
        error
            .to_string()
            .ends_with("answers.json `sleepy` is not a failure"),
        "{error}"
    );
}

#[test]
fn unknown_fields_and_formats_are_refused() {
    let folder = folder("unknown-fields");
    let typo = write(&folder, "typo.json", r#"{ "model": {} }"#);
    let format = write(
        &folder,
        "format.json",
        r#"{ "models": { "x": { "format": "telepathy" } } }"#,
    );

    assert!(matches!(
        Models::load(&[typo]),
        Err(ConfigError::Parse { .. })
    ));
    assert!(matches!(
        Models::load(&[format]),
        Err(ConfigError::Parse { .. })
    ));
}
