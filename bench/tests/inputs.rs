use chauffeur_bench::inputs::{self, Setup, Variant};
use chauffeur_bench::workspace;

#[test]
fn parses_variants() {
    let variants = inputs::parse_variants(
        r#"[
            { "id": "base", "chauffeur": false },
            { "id": "base-empty", "chauffeur": false, "disable": [] },
            { "id": "full", "chauffeur": true, "disable": [] },
            { "id": "no-rules", "chauffeur": true, "disable": ["rules", "permission"] },
            { "id": "hybrid", "chauffeur": true, "disable": [], "env": { "CHAUFFEUR_HOST_SKILLS": "keep" } },
            { "id": "short", "chauffeur": true, "disable": [], "config": "variants/short", "plugins": ["sourcefed"] }
        ]"#,
    )
    .unwrap();
    let variant = |id: &str, setup: Setup, env: &[(&str, &str)]| Variant {
        id: id.into(),
        setup,
        env: env
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect(),
        config: None,
        plugins: Vec::new(),
    };

    assert_eq!(
        variants,
        vec![
            variant("base", Setup::Base, &[]),
            variant("base-empty", Setup::Base, &[]),
            variant("full", Setup::Chauffeur { disable: vec![] }, &[]),
            variant(
                "no-rules",
                Setup::Chauffeur {
                    disable: vec!["rules".into(), "permission".into()]
                },
                &[]
            ),
            variant(
                "hybrid",
                Setup::Chauffeur { disable: vec![] },
                &[("CHAUFFEUR_HOST_SKILLS", "keep")]
            ),
            Variant {
                config: Some("variants/short".into()),
                plugins: vec!["sourcefed".into()],
                ..variant("short", Setup::Chauffeur { disable: vec![] }, &[])
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
        (
            r#"[{ "id": "x", "chauffeur": false, "config": "variants/x" }]"#,
            "config without chauffeur",
        ),
        (
            r#"[{ "id": "x", "chauffeur": true, "disable": [], "config": "../x" }]"#,
            "inside the suite",
        ),
        (
            r#"[{ "id": "x", "chauffeur": true, "disable": [], "config": "/etc" }]"#,
            "inside the suite",
        ),
        (
            r#"[{ "id": "x", "chauffeur": true, "disable": [], "plugins": ["chauffeur"] }]"#,
            "optional plugins are",
        ),
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

    let outside = inputs::parse_task_spec(
        r#"{ "prompt": "edit {outside}/a", "check": ["x"], "timeout_seconds": 1, "tags": [] }"#,
    )
    .unwrap();
    let task = inputs::Task {
        id: "t".into(),
        spec: outside,
        dir: "/nowhere/t".into(),
    };
    assert_eq!(
        task.prompt(std::path::Path::new("/run/outside")),
        "edit /run/outside/a"
    );

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
    let base = Variant::base("base");
    let full = Variant {
        setup: Setup::Chauffeur {
            disable: vec!["rules".into()],
        },
        ..Variant::base("full")
    };
    let sourcefed = Variant {
        plugins: vec!["sourcefed".into()],
        ..full.clone()
    };

    assert_eq!(
        workspace::plugin_list(&base),
        vec!["-chauffeur", "-ntfy-notify", "-optmem", "-sourcefed"]
    );
    assert_eq!(
        workspace::plugin_list(&full),
        vec!["-ntfy-notify", "-optmem", "-sourcefed"]
    );
    assert_eq!(
        workspace::plugin_list(&sourcefed),
        vec!["-ntfy-notify", "-optmem"]
    );

    let config: serde_json::Value =
        serde_json::from_str(&workspace::opencode_config(&base).unwrap()).unwrap();
    assert_eq!(
        config,
        serde_json::json!({ "plugins": ["-chauffeur", "-ntfy-notify", "-optmem", "-sourcefed"] })
    );
}

#[test]
fn load_variants_resolves_config_folders_inside_the_root() {
    let root =
        std::env::temp_dir().join(format!("chauffeur-bench-variants-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("variants/short")).unwrap();
    let write = |text: &str| std::fs::write(root.join("variants.json"), text).unwrap();

    write(r#"[{ "id": "short", "chauffeur": true, "disable": [], "config": "variants/short" }]"#);
    let variants = inputs::load_variants(&root).unwrap();
    assert_eq!(variants[0].config, Some(root.join("variants/short")));

    write(r#"[{ "id": "gone", "chauffeur": true, "disable": [], "config": "variants/gone" }]"#);
    assert!(
        inputs::load_variants(&root)
            .unwrap_err()
            .contains("is not a folder")
    );

    std::fs::remove_dir_all(&root).unwrap();
}
