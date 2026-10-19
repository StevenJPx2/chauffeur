use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{OriginalUri, State};
use axum::http::StatusCode;
use axum::routing::get;
use serde_json::{Value, json};

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct Api {
    replies: HashMap<String, (StatusCode, String)>,
    /// Every request's path and query.
    log: Vec<String>,
}

type Shared = Arc<Mutex<Api>>;

async fn reply(State(api): State<Shared>, OriginalUri(uri): OriginalUri) -> (StatusCode, String) {
    let mut api = api.lock().unwrap();

    api.log.push(uri.to_string());
    api.replies.get(uri.path()).cloned().unwrap_or((
        StatusCode::NOT_FOUND,
        json!({"code":"NOT_FOUND","detail":"no such thing"}).to_string(),
    ))
}

struct Fake {
    api: Shared,
    client: SkilldClient,
    task: tokio::task::JoinHandle<()>,
}

impl Fake {
    async fn new() -> Self {
        let api = Shared::default();
        let app = Router::new()
            .route("/{*path}", get(reply))
            .with_state(Arc::clone(&api));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client =
            SkilldClient::new(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self { api, client, task }
    }

    fn set(&self, path: &str, status: StatusCode, body: &Value) {
        self.api
            .lock()
            .unwrap()
            .replies
            .insert(format!("/api/v1/{path}"), (status, body.to_string()));
    }

    fn skill(&self, owner: &str, repository: &str, name: &str, path: &str, extra: &Value) {
        let mut body = json!({
            "owner": owner,
            "repository": repository,
            "name": name,
            "displayName": name,
            "description": format!("The {name} skill"),
            "authorName": "Author",
            "license": "MIT",
            "repositoryUrl": format!("https://github.com/{owner}/{repository}"),
            "skillPath": path,
            "sourceCommit": "2b7020416c40f285e0bdbc98802f0d758f778759",
            "sourceGone": false,
            "pushedAt": "2026-10-19T02:45:55.000Z",
            "tags": ["vue"],
            "pageUrl": format!("https://skilld.dev/gh/{owner}/{repository}/{name}"),
            "behaviors": [],
            "stars": 1,
            "likes": 0,
            "markdown": "---\nname: x\n---\nbody"
        });

        for (key, value) in extra.as_object().unwrap() {
            body[key] = value.clone();
        }

        self.set(
            &format!("skills/{owner}/{repository}/{name}"),
            StatusCode::OK,
            &body,
        );
    }

    fn requests(&self) -> Vec<String> {
        std::mem::take(&mut self.api.lock().unwrap().log)
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn selector(text: &str) -> Selector {
    text.parse().unwrap()
}

fn git(id: &str, url: &str, paths: &[&str]) -> Source {
    Source::Git {
        id: id.into(),
        url: url.into(),
        reference: "HEAD".into(),
        paths: paths.iter().map(|path| (*path).into()).collect(),
    }
}

fn config(sources: Vec<Source>) -> SourcesConfig {
    SourcesConfig {
        sources,
        ..SourcesConfig::default()
    }
}

#[test]
fn selectors_name_one_skill_or_a_repository_and_nothing_else() {
    assert_eq!(selector("nuxt/ui/nuxt-ui").name.as_deref(), Some("nuxt-ui"));
    assert_eq!(selector("emilkowalski/skills").name, None);
    assert_eq!(
        selector("vercel-labs/agent-skills/web-design-guidelines").to_string(),
        "vercel-labs/agent-skills/web-design-guidelines"
    );

    for bad in [
        "nuxt",
        "a/b/c/d",
        "-x/repo",
        "owner/..",
        "owner/repo/../x",
        "own er/repo",
        "o/r/",
        "@curator/list",
    ] {
        assert!(bad.parse::<Selector>().is_err(), "{bad}");
    }
}

#[test]
fn the_origin_is_https_or_loopback_http() {
    assert!(SkilldClient::new("https://skilld.dev").is_ok());
    assert!(SkilldClient::new("http://127.0.0.1:3000").is_ok());

    for bad in [
        "http://skilld.dev",
        "https://skilld.dev/api",
        "https://user@skilld.dev",
        "ftp://skilld.dev",
    ] {
        assert!(SkilldClient::new(bad).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn search_sends_only_the_terms_and_returns_valid_selectors_with_clean_text() {
    let fake = Fake::new().await;

    fake.set("skills", StatusCode::OK, &json!({
        "total": 3,
        "items": [
            {"name":"nuxt-ui","description":"Build UIs\nwith \u{1b}[31mNuxt UI\u{202e}","source":{"provider":"github","owner":"nuxt","repository":"ui","selector":{"type":"named-skill","name":"nuxt-ui"}},"stargazerCount":7018},
            {"name":"bad name","description":null,"source":{"provider":"github","owner":"x","repository":"y","selector":{"type":"named-skill","name":"bad"}},"stargazerCount":1},
            {"name":"nuxt","description":null,"source":{"provider":"github","owner":"onmax","repository":"nuxt-skills","selector":{"type":"named-skill","name":"nuxt"}},"stargazerCount":722}
        ]
    }));

    let found = fake.client.search("  nuxt   ui ", 200).await.unwrap();

    assert_eq!(fake.requests(), ["/api/v1/skills?q=nuxt+ui&limit=50"]);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].selector, "nuxt/ui/nuxt-ui");
    assert_eq!(
        found[0].description.as_deref(),
        Some("Build UIs with [31mNuxt UI")
    );
    assert_eq!(found[0].stars, 7018);
    assert_eq!(found[1].description, None);
    assert!(fake.client.search("   ", 5).await.is_err());
    assert!(fake.client.search(&"x".repeat(201), 5).await.is_err());
    assert!(fake.requests().is_empty());
}

#[tokio::test]
async fn a_skill_becomes_a_candidate_only_with_complete_github_provenance() {
    let fake = Fake::new().await;

    fake.skill("nuxt", "ui", "nuxt-ui", "skills/nuxt-ui/SKILL.md", &json!({
        "behaviors": [
            {"id":"shell","label":"Runs shell commands","tier":"show","total":3,"locations":[{"path":"SKILL.md","line":7}]},
            {"id":"novel","label":"Something new","tier":"mystery","total":1,"locations":[{"path":"a.md","line":null}]}
        ]
    }));

    let candidate = fake.client.skill("nuxt", "ui", "nuxt-ui").await.unwrap();

    assert_eq!(candidate.url, "https://github.com/nuxt/ui.git");
    assert_eq!(candidate.dir, "skills/nuxt-ui");
    assert_eq!(
        candidate.commit.as_deref(),
        Some("2b7020416c40f285e0bdbc98802f0d758f778759")
    );
    assert_eq!(candidate.behaviors[0].tier, Tier::Show);
    assert_eq!(candidate.behaviors[0].locations, ["SKILL.md:7"]);
    // A tier Chauffeur does not know needs approval.
    assert_eq!(candidate.behaviors[1].tier, Tier::Ask);

    fake.skill("root", "skill", "root", "SKILL.md", &json!({}));
    assert_eq!(
        fake.client
            .skill("root", "skill", "root")
            .await
            .unwrap()
            .dir,
        "."
    );

    let broken = [
        json!({"sourceGone": true}),
        json!({"repositoryUrl": "https://gitlab.com/o/r"}),
        json!({"repositoryUrl": "http://github.com/o/r"}),
        json!({"repositoryUrl": "https://github.com/o/r/tree/main"}),
        json!({"skillPath": null}),
        json!({"skillPath": "skills/x/README.md"}),
        json!({"skillPath": "../escape/SKILL.md"}),
        json!({"sourceCommit": "main"}),
    ];

    for extra in broken {
        fake.skill("o", "r", "s", "skills/s/SKILL.md", &extra);
        assert!(
            matches!(
                fake.client.skill("o", "r", "s").await,
                Err(DiscoveryError::Invalid(_))
            ),
            "{extra}"
        );
    }
}

#[tokio::test]
async fn skilld_problems_keep_their_code_and_detail() {
    let fake = Fake::new().await;
    let error = fake
        .client
        .skill("nuxt", "ui", "missing")
        .await
        .unwrap_err();

    assert!(
        matches!(
            &error,
            DiscoveryError::Api { status: 404, code, detail } if code == "NOT_FOUND" && detail == "no such thing"
        ),
        "{error}"
    );

    fake.set(
        "skills/a/b/c",
        StatusCode::TOO_MANY_REQUESTS,
        &json!({"code":"RATE_LIMITED","title":"Slow down"}),
    );
    assert_eq!(
        fake.client
            .skill("a", "b", "c")
            .await
            .unwrap_err()
            .to_string(),
        "Skilld answered 429 RATE_LIMITED: Slow down"
    );
}

#[tokio::test]
async fn a_repository_approval_follows_the_folder_holding_all_of_its_skills() {
    let fake = Fake::new().await;

    fake.set(
        "repositories/emilkowalski/skills",
        StatusCode::OK,
        &json!({
            "owner":"emilkowalski","repository":"skills",
            "skills":[{"name":"animate","skillPath":null},{"name":"break-ui","skillPath":null}]
        }),
    );
    fake.skill(
        "emilkowalski",
        "skills",
        "animate",
        "skills/animate/SKILL.md",
        &json!({}),
    );
    fake.skill(
        "emilkowalski",
        "skills",
        "break-ui",
        "skills/break-ui/SKILL.md",
        &json!({}),
    );

    let repository = selector("emilkowalski/skills");
    let candidates = fake.client.candidates(&repository).await.unwrap();
    let approval = approve(&repository, candidates, &config(Vec::new()), &[]).unwrap();

    assert_eq!(fake.requests().len(), 3);
    assert_eq!(approval.dir, "skills");
    assert_eq!(
        approval.change,
        Change::Added {
            source: "skills".into()
        }
    );
    assert_eq!(
        approval.config.sources,
        [git(
            "skills",
            "https://github.com/emilkowalski/skills.git",
            &["skills"]
        )]
    );
    assert!(approval.writes());
}

fn candidate(url: &str, dir: &str) -> Candidate {
    Candidate {
        selector: "o/r/s".into(),
        name: "s".into(),
        description: None,
        author: None,
        license: None,
        url: url.into(),
        dir: dir.into(),
        commit: None,
        pushed_at: None,
        tags: Vec::new(),
        page: String::new(),
        behaviors: Vec::new(),
    }
}

#[test]
fn approval_extends_the_source_already_following_the_repository() {
    let existing = config(vec![git(
        "nuxt-ui",
        "git@github.com:Nuxt/UI.git",
        &["skills/nuxt-ui"],
    )]);
    let approval = approve(
        &selector("nuxt/ui/other"),
        vec![candidate("https://github.com/nuxt/ui.git", "skills/other")],
        &existing,
        &[],
    )
    .unwrap();

    assert_eq!(
        approval.change,
        Change::Extended {
            source: "nuxt-ui".into()
        }
    );
    assert_eq!(
        approval.config.sources,
        [git(
            "nuxt-ui",
            "git@github.com:Nuxt/UI.git",
            &["skills/nuxt-ui", "skills/other"]
        )]
    );

    // A covered skill writes nothing.
    let covered = approve(
        &selector("nuxt/ui/nuxt-ui"),
        vec![candidate(
            "https://github.com/nuxt/ui.git",
            "skills/nuxt-ui",
        )],
        &existing,
        &[],
    )
    .unwrap();

    assert_eq!(
        covered.change,
        Change::Covered {
            source: "nuxt-ui".into()
        }
    );
    assert!(!covered.writes());

    // A parent folder replaces the paths it now covers.
    let parent = approve(
        &selector("nuxt/ui"),
        vec![
            candidate("https://github.com/nuxt/ui.git", "skills/a"),
            candidate("https://github.com/nuxt/ui.git", "skills/b"),
        ],
        &approval.config,
        &[],
    )
    .unwrap();

    assert_eq!(
        parent.config.sources,
        [git("nuxt-ui", "git@github.com:Nuxt/UI.git", &["skills"])]
    );
}

#[test]
fn approval_respects_the_source_limits_and_unique_ids() {
    let full = config(
        (0..16)
            .map(|n| {
                git(
                    &format!("s{n}"),
                    &format!("https://github.com/o/r{n}.git"),
                    &["."],
                )
            })
            .collect(),
    );
    let error = approve(
        &selector("x/y/z"),
        vec![candidate("https://github.com/x/y.git", "z")],
        &full,
        &[],
    )
    .unwrap_err();

    assert!(error.to_string().contains("maximum 16 sources"), "{error}");

    let paths: Vec<String> = (0..16).map(|n| format!("skills/{n}")).collect();
    let crowded = config(vec![git(
        "r",
        "https://github.com/o/r.git",
        &paths.iter().map(String::as_str).collect::<Vec<_>>(),
    )]);

    assert!(
        approve(
            &selector("o/r/new"),
            vec![candidate("https://github.com/o/r.git", "skills/new")],
            &crowded,
            &[]
        )
        .is_err()
    );

    let taken = config(vec![git(
        "skills",
        "https://github.com/a/skills.git",
        &["."],
    )]);
    let approval = approve(
        &selector("b/skills/x"),
        vec![candidate("https://github.com/b/skills.git", "x")],
        &taken,
        &[],
    )
    .unwrap();

    assert_eq!(
        approval.change,
        Change::Added {
            source: "b-skills".into()
        }
    );
    assert!(
        approve(
            &selector("o/r"),
            vec![
                candidate("https://github.com/o/r.git", "a"),
                candidate("https://github.com/o/fork.git", "b")
            ],
            &config(Vec::new()),
            &[]
        )
        .is_err()
    );
}

#[test]
fn ask_behaviors_block_approval_until_each_is_allowed() {
    let mut risky = candidate("https://github.com/o/r.git", "s");

    risky.behaviors = vec![
        Behavior {
            id: "remote-code".into(),
            label: String::new(),
            tier: Tier::Ask,
            total: 1,
            locations: Vec::new(),
        },
        Behavior {
            id: "credentials".into(),
            label: String::new(),
            tier: Tier::Ask,
            total: 1,
            locations: Vec::new(),
        },
        Behavior {
            id: "shell".into(),
            label: String::new(),
            tier: Tier::Show,
            total: 1,
            locations: Vec::new(),
        },
    ];

    let blocked = approve(
        &selector("o/r/s"),
        vec![risky.clone()],
        &config(Vec::new()),
        &["remote-code".into()],
    )
    .unwrap();

    assert_eq!(blocked.blocked, ["credentials"]);
    assert!(!blocked.writes());

    let allowed = approve(
        &selector("o/r/s"),
        vec![risky],
        &config(Vec::new()),
        &["remote-code".into(), "credentials".into()],
    )
    .unwrap();

    assert!(allowed.writes());
}

fn temporary() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "chauffeur-discovery-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));

    std::fs::create_dir_all(&root).unwrap();

    root
}

#[test]
fn writing_keeps_every_setting_and_writes_through_a_symlink() {
    let root = temporary();
    let real = root.join("dotfiles.json");
    let link = root.join("skill-sources.json");

    std::fs::write(&real, r#"{"refresh_seconds":3600,"sources":[{"id":"team","type":"http","url":"https://example.com/skills/"},{"id":"pinned","type":"git","url":"https://github.com/o/r.git","ref":"v1"}],"descriptions":{"review":"Use \"when\" reviewing"}}"#).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let before = read_config(&link).unwrap();
    let approval = approve(
        &selector("nuxt/ui/nuxt-ui"),
        vec![candidate(
            "https://github.com/nuxt/ui.git",
            "skills/nuxt-ui",
        )],
        &before,
        &[],
    )
    .unwrap();

    write_config(&link, &approval.config).unwrap();

    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );

    let text = std::fs::read_to_string(&real).unwrap();
    let after = read_config(&link).unwrap();

    assert_eq!(after.refresh_seconds, 3600);
    assert_eq!(after.descriptions, before.descriptions);
    assert_eq!(after.sources[..2], before.sources[..]);
    assert_eq!(
        after.sources[2],
        git("ui", "https://github.com/nuxt/ui.git", &["skills/nuxt-ui"])
    );
    assert!(text.contains(r#"    {"type":"git","id":"ui","url":"https://github.com/nuxt/ui.git","paths":["skills/nuxt-ui"]}"#), "{text}");
    assert!(text.contains(r#""ref":"v1""#));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 2);

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn approval_never_replaces_a_file_it_cannot_read() {
    let root = temporary();
    let path = root.join("skill-sources.json");

    assert_eq!(read_config(&path).unwrap(), SourcesConfig::default());
    std::fs::write(&path, r#"{"sources":[],"unknown":true}"#).unwrap();
    assert!(read_config(&path).is_err());
    std::fs::write(&path, " ".repeat(16_385)).unwrap();
    assert!(read_config(&path).is_err());

    std::fs::remove_dir_all(root).unwrap();
}
