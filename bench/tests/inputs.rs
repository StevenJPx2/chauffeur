use chauffeur_bench::inputs::{self, Setup, Variant};
use chauffeur_bench::workspace;

#[test]
fn parses_variants() {
    let variants = inputs::parse_variants(
        r#"[
            { "id": "base", "chauffeur": false },
            { "id": "base-empty", "chauffeur": false, "disable": [] },
            { "id": "full", "chauffeur": true, "disable": [] },
            { "id": "no-rules", "chauffeur": true, "disable": ["rules", "permission"] }
        ]"#,
    )
    .unwrap();

    assert_eq!(
        variants,
        vec![
            Variant {
                id: "base".into(),
                setup: Setup::Base
            },
            Variant {
                id: "base-empty".into(),
                setup: Setup::Base
            },
            Variant {
                id: "full".into(),
                setup: Setup::Chauffeur { disable: vec![] }
            },
            Variant {
                id: "no-rules".into(),
                setup: Setup::Chauffeur {
                    disable: vec!["rules".into(), "permission".into()]
                }
            },
        ]
    );
    assert_eq!(variants[3].disable_csv(), "rules,permission");
}

#[test]
fn rejects_invalid_variants() {
    let cases = [
        (
            r#"[{ "id": "full", "chauffeur": true }]"#,
            "no disable list",
        ),
        (
            r#"[{ "id": "x", "chauffeur": false, "disable": ["rules"] }]"#,
            "without chauffeur",
        ),
        (
            r#"[{ "id": "x", "chauffeur": false, "extra": 1 }]"#,
            "unknown field",
        ),
        (
            r#"[{ "id": "x", "chauffeur": false }, { "id": "x", "chauffeur": false }]"#,
            "twice",
        ),
        (r#"[{ "id": "../x", "chauffeur": false }]"#, "must be"),
        (r#"[{ "chauffeur": false }]"#, "missing field"),
    ];

    for (text, expected) in cases {
        let error = inputs::parse_variants(text).unwrap_err();
        assert!(error.contains(expected), "{text}: {error}");
    }
}

#[test]
fn parses_and_rejects_task_specs() {
    let spec = inputs::parse_task_spec(
        r#"{ "prompt": "fix it", "check": ["sh", "check.sh"], "timeout_seconds": 600, "tags": ["rust"] }"#,
    )
    .unwrap();
    assert_eq!(spec.check, vec!["sh", "check.sh"]);

    let cases = [
        (
            r#"{ "prompt": "p", "check": ["x"], "timeout_seconds": 1, "tags": [], "model": "m" }"#,
            "unknown field",
        ),
        (
            r#"{ "prompt": "p", "check": [], "timeout_seconds": 1, "tags": [] }"#,
            "check is empty",
        ),
        (
            r#"{ "prompt": "p", "check": ["x"], "timeout_seconds": 0, "tags": [] }"#,
            "zero",
        ),
        (
            r#"{ "prompt": "p", "check": ["x"], "tags": [] }"#,
            "missing field",
        ),
    ];
    for (text, expected) in cases {
        let error = inputs::parse_task_spec(text).unwrap_err();
        assert!(error.contains(expected), "{text}: {error}");
    }
}

#[test]
fn select_keeps_order_and_rejects_unknown_names() {
    let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let id = |item: &String| item.as_str().to_owned();
    let pick = |wanted: &[&str]| {
        let wanted: Vec<String> = wanted.iter().map(ToString::to_string).collect();
        inputs::select(&items, Some(&wanted), |item: &String| item.as_str(), "task")
    };

    assert_eq!(pick(&["c", "a"]).unwrap(), vec!["a", "c"]);
    assert!(pick(&["z"]).unwrap_err().contains("unknown task z"));
    assert_eq!(
        inputs::select(&items, None, |item: &String| item.as_str(), "task")
            .unwrap()
            .iter()
            .map(id)
            .collect::<Vec<_>>(),
        items
    );
}

#[test]
fn plugin_lists_per_variant() {
    assert_eq!(
        workspace::plugin_list(&Setup::Base),
        vec!["-chauffeur", "-ntfy-notify", "-sourcefed"]
    );
    assert_eq!(
        workspace::plugin_list(&Setup::Chauffeur {
            disable: vec!["rules".into()]
        }),
        vec!["-ntfy-notify", "-sourcefed"]
    );

    let config: serde_json::Value =
        serde_json::from_str(&workspace::opencode_config(&Setup::Base).unwrap()).unwrap();
    assert_eq!(
        config,
        serde_json::json!({ "plugins": ["-chauffeur", "-ntfy-notify", "-sourcefed"] })
    );
}
