use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path as RequestPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use chauffeur_core::SkillSourceState;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::*;

static NEXT: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
struct Server {
    replies: HashMap<String, (StatusCode, String)>,
    /// Every request, as `path` or `path 304`.
    log: Vec<String>,
}

type Shared = Arc<Mutex<Server>>;

struct Fixture {
    root: PathBuf,
    url: String,
    server: Shared,
    task: tokio::task::JoinHandle<()>,
}

impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "chauffeur-sources-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let server = Shared::default();
        let app = Router::new()
            .route("/{*path}", get(reply))
            .with_state(Arc::clone(&server));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        Self {
            root,
            url,
            server,
            task,
        }
    }

    fn set(&self, path: &str, status: StatusCode, body: &str) {
        self.server
            .lock()
            .unwrap()
            .replies
            .insert(path.into(), (status, body.into()));
    }

    /// One catalog skill with a reference; `version` changes when only files do.
    fn skill(&self, text: &str, version: &str) {
        self.set("index.json", StatusCode::OK, &format!(r#"{{"skills":[{{"name":"review","version":"{version}","files":["SKILL.md","references/policy.md"]}}]}}"#));
        self.set("review/SKILL.md", StatusCode::OK, &format!("---\nname: review\ndescription: >-\n  Review code\n  with the team policy\nmetadata:\n  ignored: true\n---\n{text}"));
        self.set("review/references/policy.md", StatusCode::OK, text);
    }

    fn requests(&self) -> Vec<String> {
        std::mem::take(&mut self.server.lock().unwrap().log)
    }

    fn configure(&self, sources: serde_json::Value) {
        std::fs::write(
            self.root.join("sources.json"),
            serde_json::to_vec(&json!({"refresh_seconds":60,"sources":sources})).unwrap(),
        )
        .unwrap();
    }

    fn configure_http(&self) {
        self.configure(json!([{"id":"team","type":"http","url":self.url}]));
    }

    fn store(&self) -> SourceStore {
        SourceStore::new(self.root.join("sources.json"), self.root.join("cache")).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

/// Serves `replies`, with a content ETag that answers 304 when unchanged.
async fn reply(
    State(server): State<Shared>,
    RequestPath(path): RequestPath<String>,
    headers: HeaderMap,
) -> (StatusCode, HeaderMap, String) {
    let mut server = server.lock().unwrap();
    let (status, body) = server
        .replies
        .get(&path)
        .cloned()
        .unwrap_or((StatusCode::NOT_FOUND, String::new()));
    let etag = format!(
        "\"{}-{}\"",
        body.len(),
        body.bytes().map(u64::from).sum::<u64>()
    );
    let mut reply = HeaderMap::new();

    reply.insert("etag", etag.parse().unwrap());

    if status == StatusCode::OK
        && headers
            .get("if-none-match")
            .is_some_and(|value| value == etag.as_str())
    {
        server.log.push(format!("{path} 304"));
        return (StatusCode::NOT_MODIFIED, reply, String::new());
    }

    server.log.push(path);
    (status, reply, body)
}

#[tokio::test]
async fn http_checks_are_conditional_and_files_wait_for_attach() {
    let fixture = Fixture::new().await;
    fixture.configure_http();
    fixture.skill("first", "1");
    let mut store = fixture.store();

    assert!(store.restore().await.unwrap().skills.is_empty());
    assert!(fixture.requests().is_empty());
    let first = store.refresh(1000).await.unwrap();
    let skill = &first.skills[0];
    let base = Path::new(&skill.path).parent().unwrap();

    // Descriptions only: the reference is listed but not downloaded.
    assert_eq!(fixture.requests(), ["index.json", "review/SKILL.md"]);
    assert_eq!(skill.description, "Review code with the team policy");
    assert!(skill.content.contains("Source: team"));
    assert!(skill.content.contains(&format!(
        "- {}",
        base.join("references/policy.md").display()
    )));
    assert!(skill.content.ends_with("first"));
    assert!(!base.exists());

    store.materialize(&["review".into()]).await.unwrap();
    assert_eq!(fixture.requests(), ["review/references/policy.md"]);
    assert_eq!(
        std::fs::read_to_string(base.join("references/policy.md")).unwrap(),
        "first"
    );
    assert!(
        std::fs::read_to_string(&skill.path)
            .unwrap()
            .contains("first")
    );
    assert!(
        std::fs::metadata(base.join("references/policy.md"))
            .unwrap()
            .permissions()
            .readonly()
    );
    // Already on disk: attaching again fetches nothing.
    store.materialize(&["review".into()]).await.unwrap();
    assert!(fixture.requests().is_empty());

    // Not due yet, then due but unchanged: one conditional request, no download.
    store.refresh(1059).await.unwrap();
    assert!(fixture.requests().is_empty());
    let unchanged = store.refresh(1060).await.unwrap();
    assert_eq!(fixture.requests(), ["index.json 304"]);
    assert_eq!(unchanged.skills[0].path, skill.path);
    assert!(matches!(
        unchanged.sources[0].state,
        SkillSourceState::Ready {
            checked_at: 1060,
            ..
        }
    ));

    fixture.skill("second", "2");
    let updated = store.refresh(1120).await.unwrap();

    assert_ne!(updated.skills[0].path, skill.path);
    assert!(updated.skills[0].content.ends_with("second"));
    store.materialize(&["review".into()]).await.unwrap();
    // The previous revision stays for sessions still using it.
    assert_eq!(
        std::fs::read_to_string(base.join("references/policy.md")).unwrap(),
        "first"
    );
}

#[tokio::test]
async fn failed_checks_keep_the_last_good_listing_and_restore_offline() {
    let fixture = Fixture::new().await;
    fixture.configure_http();
    fixture.skill("good", "1");
    let mut store = fixture.store();
    let first = store.refresh(1000).await.unwrap();

    fixture.set("index.json", StatusCode::SERVICE_UNAVAILABLE, "down");
    let failed = store.refresh(1060).await.unwrap();

    assert_eq!(failed.skills, first.skills);
    assert!(matches!(
        failed.sources[0].state,
        SkillSourceState::Stale {
            checked_at: 1000,
            ..
        }
    ));
    fixture.skill("partial", "2");
    fixture.set("review/SKILL.md", StatusCode::OK, "no frontmatter");
    assert_eq!(store.refresh(1120).await.unwrap().skills, first.skills);
    fixture.set(
        "index.json",
        StatusCode::OK,
        r#"{"skills":[{"name":"review","files":["SKILL.md","../outside"]}]}"#,
    );
    assert_eq!(store.refresh(1180).await.unwrap().skills, first.skills);

    fixture.task.abort();
    let mut restored = fixture.store();

    assert_eq!(restored.restore().await.unwrap().skills, first.skills);
    assert_eq!(restored.refresh(1240).await.unwrap().skills, first.skills);
}

#[tokio::test]
async fn a_failed_file_fetch_publishes_nothing_and_an_oversized_file_is_refused() {
    let fixture = Fixture::new().await;
    fixture.configure_http();
    fixture.skill("good", "1");
    let mut store = fixture.store();
    let first = store.refresh(1000).await.unwrap();
    let base = Path::new(&first.skills[0].path)
        .parent()
        .unwrap()
        .to_path_buf();

    fixture.set(
        "review/references/policy.md",
        StatusCode::SERVICE_UNAVAILABLE,
        "",
    );
    assert!(store.materialize(&["review".into()]).await.is_err());
    assert!(!base.exists());
    fixture.set(
        "review/references/policy.md",
        StatusCode::OK,
        &"x".repeat(MAX_FILE + 1),
    );
    assert!(store.materialize(&["review".into()]).await.is_err());
    assert!(!base.exists());
    // No staging folders survive a failure.
    assert_eq!(
        std::fs::read_dir(base.parent().unwrap()).unwrap().count(),
        0
    );
}

#[tokio::test]
async fn your_description_replaces_upstreams_without_a_refetch() {
    let fixture = Fixture::new().await;
    fixture.skill("good", "1");
    fixture.configure_http();
    let mut store = fixture.store();

    store.refresh(1000).await.unwrap();
    fixture.requests();
    std::fs::write(
        fixture.root.join("sources.json"),
        serde_json::to_vec(&json!({
            "refresh_seconds": 60,
            "sources": [{"id":"team","type":"http","url":fixture.url}],
            "descriptions": {"review": "Use when reviewing a pull request."}
        }))
        .unwrap(),
    )
    .unwrap();
    let catalog = store.refresh(1010).await.unwrap();

    assert_eq!(
        catalog.skills[0].description,
        "Use when reviewing a pull request."
    );
    assert!(catalog.skills[0].content.ends_with("good"));
    assert!(
        fixture.requests().is_empty(),
        "editing a description refetches nothing"
    );
}

#[tokio::test]
async fn invalid_configuration_preserves_the_offer_and_removal_withdraws_it() {
    let fixture = Fixture::new().await;
    fixture.configure_http();
    fixture.skill("good", "1");
    let mut store = fixture.store();
    let first = store.refresh(1000).await.unwrap();

    store.materialize(&["review".into()]).await.unwrap();
    std::fs::write(fixture.root.join("sources.json"), "invalid JSON").unwrap();
    assert!(store.refresh(1060).await.is_err());
    assert_eq!(store.catalog().skills, first.skills);
    fixture.configure(json!([]));
    assert!(store.refresh(1120).await.unwrap().skills.is_empty());
    assert!(Path::new(&first.skills[0].path).is_file());
}

#[tokio::test]
async fn one_broken_source_does_not_block_another_and_source_order_breaks_collisions() {
    let fixture = Fixture::new().await;
    fixture.skill("good", "1");
    fixture.configure(json!([
        {"id":"missing","type":"http","url":format!("{}missing/",fixture.url)},
        {"id":"team","type":"http","url":fixture.url},
        {"id":"copy","type":"http","url":fixture.url}
    ]));
    let mut store = fixture.store();
    let catalog = store.refresh(1000).await.unwrap();

    assert_eq!(catalog.skills.len(), 1);
    assert_eq!(catalog.skills[0].source_id, "team");
    assert!(matches!(
        catalog.sources[0].state,
        SkillSourceState::Unavailable { .. }
    ));
    assert!(matches!(
        catalog.sources[1].state,
        SkillSourceState::Ready { .. }
    ));
}

#[test]
fn configuration_rejects_unknown_fields_untrusted_protocols_secrets_and_escaping_paths() {
    for source in [
        json!({"id":"team","type":"http","url":"https://example.com/?token=secret"}),
        json!({"id":"team","type":"http","url":"https://secret@example.com/"}),
        json!({"id":"team","type":"http","url":"http://example.com/"}),
        json!({"id":"team","type":"git","url":"file:///etc"}),
        json!({"id":"team","type":"git","url":"https://example.com/repo","ref":"--exec"}),
        json!({"id":"team","type":"git","url":"https://example.com/repo","paths":["../skills"]}),
        json!({"id":"team","type":"git","url":"https://example.com/repo","typo":true}),
    ] {
        let parsed = serde_json::from_value::<SourcesConfig>(json!({"sources":[source]}));

        assert!(
            parsed
                .map_err(SourceError::from)
                .and_then(SourcesConfig::check)
                .is_err()
        );
    }

    for path in [
        "../x",
        "/x",
        "x//y",
        "x\\y",
        "x/%2e%2e/y",
        ".git/config",
        ".repository/config",
        "https://evil/x",
    ] {
        assert!(!valid_path(path));
    }

    let ssh =
        json!({"sources":[{"id":"ssh","type":"git","url":"git@example.com:team/skills.git"}]});
    let config = serde_json::from_value::<SourcesConfig>(ssh)
        .unwrap()
        .check()
        .unwrap();

    assert_eq!(config.refresh_seconds, 86_400);
}

#[test]
fn metadata_accepts_crlf_quoted_and_folded_descriptions_but_rejects_duplicates() {
    let parsed = parse::skill(
        b"\xef\xbb\xbf---\r\nname: review\r\ndescription: 'Review changes'\r\n---\r\nbody",
    )
    .unwrap();

    assert_eq!(parsed.description, "Review changes");
    assert_eq!(parsed.body, "body");
    assert!(
        parse::skill(b"---\nname: review\nname: duplicate\ndescription: x\n---\nbody").is_err()
    );
    assert!(parse::skill(b"---\nname: review\ndescription: x\n---not-a-delimiter\nbody").is_err());
}

/// Kills a whole process group on drop, including after a failed assertion.
pub(super) struct KillGroup(u32);

impl Drop for KillGroup {
    fn drop(&mut self) {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", &format!("-{}", self.0)])
            .status();
    }
}

pub(super) async fn git(repository: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=Test",
        ])
        .arg("-C")
        .arg(repository)
        .args(args)
        .output()
        .await
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout).unwrap().trim().into()
}

/// Whether the partial clone holds `object` without fetching it.
async fn present(repository: &Path, object: &str) -> bool {
    Command::new("git")
        .arg("--git-dir")
        .arg(repository)
        .args(["cat-file", "-e", object])
        .env("GIT_NO_LAZY_FETCH", "1")
        .status()
        .await
        .unwrap()
        .success()
}

/// A `git-daemon` serving `root`, counting connections.
pub(super) async fn serve_git(
    root: &Path,
) -> (
    u16,
    KillGroup,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let exec_path = git(root, &["--exec-path"]).await;
    // git-daemon forks per connection; its own process group lets the guard kill every fork.
    let mut daemon = Command::new(Path::new(&exec_path).join("git-daemon"))
        .args(["--verbose", "--export-all", "--listen=127.0.0.1"])
        .arg(format!("--port={port}"))
        .arg(format!("--base-path={}", root.display()))
        .arg(root)
        .stderr(Stdio::piped())
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let group = KillGroup(daemon.id().unwrap());
    let mut stderr = BufReader::new(daemon.stderr.take().unwrap());
    let mut ready = String::new();

    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stderr.read_line(&mut ready),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(ready.contains("Ready to rumble"), "{ready}");
    let connections = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&connections);
    // Keep draining per-connection logs so a full pipe can never stall the server.
    let drain = tokio::spawn(async move {
        let _daemon = daemon;
        let mut line = String::new();

        while stderr.read_line(&mut line).await.is_ok_and(|read| read > 0) {
            if line.contains("Connection from") {
                counted.fetch_add(1, Ordering::Relaxed);
            }

            line.clear();
        }
    });

    (port, group, connections, drain)
}

#[tokio::test]
async fn git_checks_are_one_request_and_scripts_arrive_executable_on_attach() {
    let fixture = Fixture::new().await;
    let repository = fixture.root.join("repo");
    std::fs::create_dir_all(repository.join("skills/review/scripts")).unwrap();
    std::fs::create_dir_all(repository.join("skills/other")).unwrap();
    git(&repository, &["init", "-b", "main"]).await;
    // Partial clone serving needs filters, and lazy blob fetches need wants by object name.
    git(&repository, &["config", "uploadpack.allowFilter", "true"]).await;
    git(
        &repository,
        &["config", "uploadpack.allowAnySHA1InWant", "true"],
    )
    .await;
    std::fs::write(
        repository.join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: Review changes\n---\nfirst",
    )
    .unwrap();
    std::fs::write(
        repository.join("skills/review/scripts/check.sh"),
        "#!/bin/sh\necho checked\n",
    )
    .unwrap();
    std::fs::write(
        repository.join("skills/other/SKILL.md"),
        "---\nname: other\ndescription: Other work\n---\nother",
    )
    .unwrap();
    std::fs::write(repository.join("skills/other/data.txt"), "unused").unwrap();
    std::fs::write(repository.join("outside.txt"), "not selected").unwrap();
    std::fs::create_dir_all(repository.join("skills/review/references")).unwrap();

    for name in ["a", "b", "c"] {
        std::fs::write(
            repository.join(format!("skills/review/references/{name}.md")),
            format!("reference {name}"),
        )
        .unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let script = repository.join("skills/review/scripts/check.sh");

        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::os::unix::fs::symlink("/etc/passwd", repository.join("skills/review/secret")).unwrap();
    }
    git(&repository, &["add", "."]).await;
    git(&repository, &["commit", "-m", "first"]).await;
    let revision = git(&repository, &["rev-parse", "HEAD"]).await;
    let script = git(
        &repository,
        &["rev-parse", "HEAD:skills/review/scripts/check.sh"],
    )
    .await;
    let unused = git(&repository, &["rev-parse", "HEAD:skills/other/data.txt"]).await;
    let (port, group, connections, drain) = serve_git(&fixture.root).await;
    fixture.configure(json!([{"id":"repo","type":"git","url":format!("git://127.0.0.1:{port}/repo"),"ref":"main","paths":["skills"]}]));
    let clone = fixture.root.join("cache/repo/repo.git");
    let mut store = fixture.store();
    let first = store.refresh(1000).await.unwrap();

    // One ref check, one tree fetch, and one batch for both SKILL.md files.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(connections.swap(0, Ordering::Relaxed), 3);
    let review = first
        .skills
        .iter()
        .find(|skill| skill.id == "review")
        .unwrap();
    let base = Path::new(&review.path).parent().unwrap();

    assert_eq!(first.skills.len(), 2, "{first:?}");
    assert_eq!(review.revision, revision);
    assert!(review.content.contains(&format!(
        "- {} (executable)",
        base.join("scripts/check.sh").display()
    )));
    assert!(!review.content.contains("secret"));
    // Descriptions only: neither skill's other files were downloaded.
    assert!(!present(&clone, &script).await);
    assert!(!present(&clone, &unused).await);

    store.materialize(&["review".into()]).await.unwrap();
    // All four of the skill's files arrive in one request.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(connections.load(Ordering::Relaxed), 1);
    assert_eq!(
        std::fs::read_to_string(base.join("references/c.md")).unwrap(),
        "reference c"
    );
    let checked = base.join("scripts/check.sh");
    let output = std::process::Command::new(&checked).output().unwrap();

    assert_eq!(String::from_utf8(output.stdout).unwrap(), "checked\n");
    assert!(
        std::fs::metadata(&checked)
            .unwrap()
            .permissions()
            .readonly()
    );
    assert!(!base.join("secret").exists());
    assert!(
        !present(&clone, &unused).await,
        "an unattached skill's files stay remote"
    );

    // Unchanged upstream: one connection for the ref check, nothing fetched.
    connections.store(0, Ordering::Relaxed);
    let unchanged = store.refresh(1060).await.unwrap();

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(connections.load(Ordering::Relaxed), 1);
    assert_eq!(unchanged.skills, first.skills);

    std::fs::write(
        repository.join("skills/review/SKILL.md"),
        "---\nname: review\ndescription: Review changes\n---\nsecond",
    )
    .unwrap();
    git(&repository, &["commit", "-am", "second"]).await;
    let second = store.refresh(1120).await.unwrap();
    let updated = second
        .skills
        .iter()
        .find(|skill| skill.id == "review")
        .unwrap();

    assert_ne!(updated.revision, revision);
    assert!(updated.content.ends_with("second"));
    store.materialize(&["review".into()]).await.unwrap();
    assert!(Path::new(&updated.path).is_file());
    // The first revision stays for sessions still using it.
    assert!(
        std::fs::read_to_string(&review.path)
            .unwrap()
            .contains("first")
    );

    drop(group);
    drain.await.unwrap();
}
