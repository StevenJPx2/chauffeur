//! A cold catalog and its updates cross real HTTP, daemon RPC, Rust client, and CLI boundaries.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chauffeur_core::{CatalogEntry, DaemonClient, Effect, Signal, SignalKind, SkillsResult};
use serde_json::json;

struct Fixture {
    root: PathBuf,
    source: String,
    stop: Arc<AtomicBool>,
    server: Option<std::thread::JoinHandle<()>>,
    daemon: Option<Child>,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("chauffeur-skills-rpc-{}", std::process::id()));
        std::fs::create_dir_all(root.join("config")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let source = format!("http://{}/", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&stop);

        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => respond(stream),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("source server: {error}"),
                }
            }
        });

        Self {
            root,
            source,
            stop,
            server: Some(server),
            daemon: None,
        }
    }

    fn configure(&self, url: &str) {
        std::fs::write(
            self.root.join("config/skill-sources.json"),
            serde_json::to_vec(&json!({
                "refresh_seconds":60,"sources":[{"id":"team","type":"http","url":url}]
            }))
            .unwrap(),
        )
        .unwrap();
    }

    fn start(&mut self) -> String {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let daemon = Command::new(env!("CARGO_BIN_EXE_chauffeur"))
            .args(["daemon", "--port", &port.to_string()])
            .env("TYPESAFE_API_KEY", "test")
            // The same fixture server answers as the judge.
            .env("TYPESAFE_BASE_URL", self.source.trim_end_matches('/'))
            .env("CHAUFFEUR_JUDGE", "jev")
            .env("CHAUFFEUR_JUDGE_FALLBACK", "none")
            .env("CHAUFFEUR_CONFIG_DIR", self.root.join("config"))
            .env("CHAUFFEUR_STATE_DIR", self.root.join("state"))
            .env(
                "CHAUFFEUR_SKILLS_DIR",
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills"),
            )
            .env("CHAUFFEUR_SOURCEFED", "off")
            .env_remove("CHAUFFEUR_DAEMON_TOKEN")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        self.daemon = Some(daemon);

        format!("http://127.0.0.1:{port}")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(mut daemon) = self.daemon.take() {
            let _ = daemon.kill();
            let _ = daemon.wait();
        }

        self.stop.store(true, Ordering::Relaxed);

        if let Some(server) = self.server.take() {
            server.join().unwrap();
        }

        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

/// The request line and the body, read whole by its `Content-Length`.
fn read_request(stream: &mut TcpStream) -> (String, Vec<u8>) {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    let mut length = 0;

    reader.read_line(&mut first).unwrap();

    loop {
        let mut line = String::new();

        reader.read_line(&mut line).unwrap();

        if line == "\r\n" || line.is_empty() {
            break;
        }

        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap();
        }
    }

    let mut body = vec![0; length];

    reader.read_exact(&mut body).unwrap();

    (first, body)
}

/// Answers every judge question with a confident yes.
fn judge(body: &[u8]) -> String {
    let request: serde_json::Value = serde_json::from_slice(body).unwrap();
    let answers: serde_json::Map<String, serde_json::Value> = request["questions"]
        .as_object()
        .unwrap()
        .keys()
        .map(|id| {
            (
                id.clone(),
                json!({"type":"noul","noul":0.95,"confidence":0.9}),
            )
        })
        .collect();

    json!({ "answers": answers }).to_string()
}

fn respond(mut stream: TcpStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let (first, request) = read_request(&mut stream);
    let mut words = first.split_whitespace();
    let method = words.next().unwrap();
    let path = words.next().unwrap();
    let version = if path.starts_with("/v2/") {
        "second"
    } else {
        "first"
    };
    let body = if method == "POST" {
        judge(&request)
    } else if path.ends_with("index.json") {
        r#"{"skills":[{"name":"review","files":["SKILL.md","references/policy.md"]}]}"#.into()
    } else if path.ends_with("SKILL.md") {
        format!(
            "---\nname: review\ndescription: >-\n  Review code\n  with the team policy\n---\n{version} remote instructions"
        )
    } else {
        format!("{version} policy")
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );

    stream.write_all(response.as_bytes()).unwrap();
}

#[tokio::test]
async fn a_configured_remote_skill_is_available_to_clients_and_hot_reloads_without_a_host() {
    let mut fixture = Fixture::new();
    fixture.configure(&fixture.source);
    let address = fixture.start();
    let client = DaemonClient::new(&address, None).unwrap();

    for _ in 0..60 {
        if client.health().await.is_ok() {
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let first = client.skills().await.unwrap();

    assert_eq!(first.skills.len(), 1);
    assert_eq!(
        first.skills[0].description,
        "Review code with the team policy"
    );
    assert!(
        first.skills[0]
            .content
            .contains("first remote instructions")
    );
    // Files arrive when a skill is attached, not when it is listed.
    assert!(!PathBuf::from(&first.skills[0].path).exists());
    assert!(first.skills[0].content.contains("references/policy.md"));
    let cli = Command::new(env!("CARGO_BIN_EXE_chauffeur"))
        .arg("skills")
        .env("CHAUFFEUR_DAEMON_URL", &address)
        .env_remove("CHAUFFEUR_DAEMON_TOKEN")
        .output()
        .unwrap();

    assert!(cli.status.success());
    assert_eq!(
        serde_json::from_slice::<SkillsResult>(&cli.stdout).unwrap(),
        first
    );

    // A prompt the judge attaches the skill to: its files are on disk when the reply returns.
    let effects = client
        .signal(Signal {
            agent_id: "ses_skills".into(),
            at: 1,
            kind: SignalKind::UserMessage {
                text: "review this change".into(),
                first_in_context: true,
                skills: vec![CatalogEntry {
                    id: "review".into(),
                    description: first.skills[0].description.clone(),
                    bytes: 2_000,
                }],
                tools: Vec::new(),
                model: None,
                code_mode: Vec::new(),
                workspace: String::new(),
            },
        })
        .await
        .unwrap();
    let base = PathBuf::from(&first.skills[0].path)
        .parent()
        .unwrap()
        .to_path_buf();

    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::Context { skills, .. } if skills == &["review".to_string()]
        )),
        "{effects:?}"
    );
    assert_eq!(
        std::fs::read_to_string(base.join("references/policy.md")).unwrap(),
        "first policy"
    );
    assert!(
        std::fs::read_to_string(base.join("SKILL.md"))
            .unwrap()
            .contains("first remote instructions")
    );
    fixture.configure(&format!("{}v2/", fixture.source));
    let mut current = first.clone();

    for _ in 0..80 {
        current = client.skills().await.unwrap();

        if current
            .skills
            .first()
            .is_some_and(|skill| skill.content.contains("second remote instructions"))
        {
            break;
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert!(
        current.skills[0]
            .content
            .contains("second remote instructions")
    );
    assert_ne!(current.skills[0].path, first.skills[0].path);
}
