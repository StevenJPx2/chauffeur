//! Orders a prompt's skills by meaning with a local embedding service, so the
//! skill judge asks about the closest few instead of every skill. This is the
//! I/O half only: which skills are judged is skill-exposure's policy. Any
//! failure, or a cold cache, leaves the ranking empty and the judge asks about
//! every skill up to its cap, as without a service.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chauffeur_core::{CatalogEntry, Signal, SignalKind};
use serde::Deserialize;
use serde_json::json;

/// The prompt waits at most this for its ranking; the service normally takes about 0.1 s.
const QUERY_DEADLINE: Duration = Duration::from_millis(400);
/// Embedding a whole catalog runs in the background, bounded by this.
const FILL_DEADLINE: Duration = Duration::from_secs(60);
/// The service's batch limit.
const BATCH: usize = 32;
/// Characters of request text embedded, the service's own budget.
const QUERY_CHARS: usize = 2_000;
const MAX_AGENTS: usize = 256;
const MAX_VECTORS: usize = 1_024;

/// `None` when ranking is off (`CHAUFFEUR_EMBEDDINGS=off`).
#[derive(Clone)]
pub(super) struct Ranker(Option<Arc<Inner>>);

struct Inner {
    url: String,
    client: reqwest::Client,
    state: Mutex<State>,
    /// One background fill at a time.
    filling: AtomicBool,
}

#[derive(Default)]
struct State {
    /// The service's model identity, which every embed request must name.
    model: Option<String>,
    /// Each skill's vector, keyed by its ID and description.
    vectors: HashMap<u64, Arc<[f32]>>,
    /// Each agent's latest prompt catalog, for ranking its later requests.
    catalogs: HashMap<String, Vec<CatalogEntry>>,
    /// Each agent's previous prompt, so a short follow-up keeps its topic.
    previous: HashMap<String, String>,
}

#[derive(Deserialize)]
struct Health {
    model: String,
}

#[derive(Deserialize)]
struct Embedded {
    vectors: Vec<Vec<f32>>,
}

impl Ranker {
    /// A ranker for the loopback service at `url`, or none.
    ///
    /// # Errors
    /// When `url` is not a loopback HTTP address: prompts are sent there unredacted.
    pub(super) fn new(url: Option<&str>) -> Result<Self, String> {
        let Some(url) = url else {
            return Ok(Self(None));
        };
        let parsed =
            reqwest::Url::parse(url).map_err(|error| format!("embeddings URL: {error}"))?;

        if !matches!(parsed.scheme(), "http" | "https")
            || !matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
        {
            return Err("CHAUFFEUR_EMBEDDINGS must be a loopback http URL, or off".into());
        }

        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| format!("embeddings client: {error}"))?;

        Ok(Self(Some(Arc::new(Inner {
            url: url.trim_end_matches('/').to_owned(),
            client,
            state: Mutex::new(State::default()),
            filling: AtomicBool::new(false),
        }))))
    }

    /// Fill a prompt's or agent request's `ranking`, closest skill first.
    pub(super) async fn rank(&self, signal: &mut Signal) {
        let Some(inner) = &self.0 else {
            return;
        };
        let Some((query, catalog)) = inner.subject(signal) else {
            return;
        };
        let Ok(Some(order)) =
            tokio::time::timeout(QUERY_DEADLINE, inner.order(&query, &catalog)).await
        else {
            return;
        };

        match &mut signal.kind {
            SignalKind::UserMessage { ranking, .. } | SignalKind::AgentRequest { ranking, .. } => {
                *ranking = order;
            }
            _ => {}
        }
    }
}

impl Inner {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The text to rank by and the skills to rank, remembering a prompt's
    /// catalog and text for the agent's later requests and follow-ups.
    fn subject(&self, signal: &Signal) -> Option<(String, Vec<CatalogEntry>)> {
        let mut state = self.state();

        if state.catalogs.len() >= MAX_AGENTS && !state.catalogs.contains_key(&signal.agent_id) {
            state.catalogs.clear();
            state.previous.clear();
        }

        let (query, catalog) = match &signal.kind {
            SignalKind::UserMessage { text, skills, .. } => {
                let previous = state
                    .previous
                    .insert(signal.agent_id.clone(), text.clone())
                    .unwrap_or_default();

                state
                    .catalogs
                    .insert(signal.agent_id.clone(), skills.clone());
                (format!("{previous}\n{text}"), skills.clone())
            }
            SignalKind::AgentRequest {
                need, user_request, ..
            } => (
                format!("{user_request}\n{need}"),
                state.catalogs.get(&signal.agent_id)?.clone(),
            ),
            _ => return None,
        };
        // The latest words matter most, so a long query keeps its end.
        let skip = query.chars().count().saturating_sub(QUERY_CHARS);
        let query: String = query.trim().chars().skip(skip).collect();

        (!catalog.is_empty() && !query.is_empty()).then_some((query, catalog))
    }

    /// Skill IDs closest first, or `None` while the catalog is still being embedded.
    async fn order(self: &Arc<Self>, query: &str, catalog: &[CatalogEntry]) -> Option<Vec<String>> {
        let model = self.model().await?;
        let (known, missing): (Vec<_>, Vec<_>) = {
            let state = self.state();

            catalog
                .iter()
                .map(|skill| (skill, state.vectors.get(&key(skill)).cloned()))
                .partition(|(_, vector)| vector.is_some())
        };

        if !missing.is_empty() {
            self.fill(
                model,
                missing
                    .into_iter()
                    .map(|(skill, _)| skill.clone())
                    .collect(),
            );
            return None;
        }

        let query = self
            .embed(&model, "query", vec![query.to_owned()])
            .await?
            .pop()?;
        let mut scored: Vec<(f32, String)> = known
            .into_iter()
            .filter_map(|(skill, vector)| Some((dot(&query, &vector?), skill.id.clone())))
            .collect();

        scored.sort_by(|a, b| b.0.total_cmp(&a.0));

        Some(scored.into_iter().map(|(_, id)| id).collect())
    }

    async fn model(&self) -> Option<String> {
        if let Some(model) = self.state().model.clone() {
            return Some(model);
        }

        let health: Health = self
            .client
            .get(format!("{}/health", self.url))
            .send()
            .await
            .ok()?
            .json()
            .await
            .ok()?;

        self.state().model = Some(health.model.clone());

        Some(health.model)
    }

    /// Embed `skills` in the background; a prompt meanwhile judges unranked.
    fn fill(self: &Arc<Self>, model: String, skills: Vec<CatalogEntry>) {
        if self.filling.swap(true, Ordering::AcqRel) {
            return;
        }

        let inner = Arc::clone(self);

        // Owned by its deadline: it ends within FILL_DEADLINE or with the daemon.
        tokio::spawn(async move {
            let _ = tokio::time::timeout(FILL_DEADLINE, async {
                for batch in skills.chunks(BATCH) {
                    let texts = batch.iter().map(document).collect();
                    let Some(vectors) = inner.embed(&model, "document", texts).await else {
                        return;
                    };
                    let mut state = inner.state();

                    if state.vectors.len().saturating_add(batch.len()) > MAX_VECTORS {
                        state.vectors.clear();
                    }

                    for (skill, vector) in batch.iter().zip(vectors) {
                        state.vectors.insert(key(skill), vector.into());
                    }
                }
            })
            .await;

            inner.filling.store(false, Ordering::Release);
        });
    }

    async fn embed(&self, model: &str, kind: &str, texts: Vec<String>) -> Option<Vec<Vec<f32>>> {
        let count = texts.len();
        let response = self
            .client
            .post(format!("{}/embed", self.url))
            .json(&json!({ "model": model, "kind": kind, "texts": texts }))
            .send()
            .await
            .ok()?;

        // A restarted service may run another model: learn it, and drop vectors from the old one.
        if response.status() == reqwest::StatusCode::CONFLICT {
            let mut state = self.state();

            state.model = None;
            state.vectors.clear();
            return None;
        }

        let embedded: Embedded = response.error_for_status().ok()?.json().await.ok()?;
        let width = embedded.vectors.first().map(Vec::len)?;

        (embedded.vectors.len() == count
            && width > 0
            && embedded.vectors.iter().all(|vector| {
                vector.len() == width && vector.iter().all(|value| value.is_finite())
            }))
        .then_some(embedded.vectors)
    }
}

/// A skill as the service embeds it: its name and the description the judge reads.
fn document(skill: &CatalogEntry) -> String {
    format!("{}: {}", skill.id, skill.description)
}

/// A skill's cache key; an edited description is embedded again.
fn key(skill: &CatalogEntry) -> u64 {
    let mut hasher = DefaultHasher::new();

    skill.id.hash(&mut hasher);
    skill.description.hash(&mut hasher);
    hasher.finish()
}

/// The service returns normalized vectors, so their dot product is their cosine similarity.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use axum::Json;
    use axum::Router;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::{get, post};
    use serde_json::Value;

    use super::*;

    /// A fake service: each text's vector is its similarity to three topics.
    async fn service(calls: Arc<AtomicUsize>) -> String {
        async fn embed(
            State(calls): State<Arc<AtomicUsize>>,
            Json(body): Json<Value>,
        ) -> (StatusCode, Json<Value>) {
            calls.fetch_add(1, Ordering::Relaxed);

            if body["model"] != "test-model" {
                return (StatusCode::CONFLICT, Json(json!({})));
            }

            let vectors: Vec<Vec<f32>> = body["texts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|text| {
                    let text = text.as_str().unwrap();

                    ["test", "slack", "deploy"]
                        .iter()
                        .map(|topic| if text.contains(topic) { 1.0 } else { 0.0 })
                        .collect()
                })
                .collect();

            (StatusCode::OK, Json(json!({ "vectors": vectors })))
        }

        let app = Router::new()
            .route(
                "/health",
                get(|| async { Json(json!({ "model": "test-model" })) }),
            )
            .route("/embed", post(embed))
            .with_state(calls);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());

        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        url
    }

    fn skill(id: &str, description: &str) -> CatalogEntry {
        CatalogEntry {
            id: id.into(),
            description: description.into(),
            bytes: 100,
        }
    }

    fn prompt(text: &str) -> Signal {
        Signal {
            agent_id: "ses".into(),
            at: 1,
            kind: SignalKind::UserMessage {
                text: text.into(),
                first_in_context: false,
                skills: vec![
                    skill("vitest", "Write and run unit test suites"),
                    skill("slack-cli", "Read and post slack messages"),
                    skill("wrangler", "deploy Workers"),
                ],
                tools: Vec::new(),
                model: None,
                code_mode: Vec::new(),
                workspace: String::new(),
                ranking: Vec::new(),
            },
        }
    }

    fn ranking(signal: &Signal) -> &[String] {
        match &signal.kind {
            SignalKind::UserMessage { ranking, .. } | SignalKind::AgentRequest { ranking, .. } => {
                ranking
            }
            _ => &[],
        }
    }

    /// Rank `signal` once the background fill has finished.
    async fn ranked(ranker: &Ranker, mut signal: Signal) -> Signal {
        for _ in 0..50 {
            let mut attempt = signal.clone();

            ranker.rank(&mut attempt).await;

            if !ranking(&attempt).is_empty() {
                return attempt;
            }

            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        ranker.rank(&mut signal).await;
        signal
    }

    #[tokio::test]
    async fn a_cold_catalog_judges_unranked_then_ranks_by_meaning() {
        let calls = Arc::new(AtomicUsize::new(0));
        let ranker = Ranker::new(Some(&service(Arc::clone(&calls)).await)).unwrap();
        let mut cold = prompt("post the deploy notes to slack");

        ranker.rank(&mut cold).await;
        assert!(
            ranking(&cold).is_empty(),
            "the first prompt falls back while skills embed"
        );

        let warm = ranked(&ranker, prompt("post the deploy notes to slack")).await;

        assert_eq!(ranking(&warm)[2], "vitest");
        // Skill vectors are cached: a later prompt embeds only its own text.
        calls.store(0, Ordering::Relaxed);
        let _ = ranked(&ranker, prompt("add a test")).await;
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn a_follow_up_keeps_the_previous_prompts_topic_and_requests_use_the_catalog() {
        let ranker = Ranker::new(Some(&service(Arc::new(AtomicUsize::new(0))).await)).unwrap();
        let _ = ranked(&ranker, prompt("add a test for the parser")).await;
        let follow_up = ranked(&ranker, prompt("yes, do it")).await;

        assert_eq!(ranking(&follow_up)[0], "vitest");

        let request = ranked(
            &ranker,
            Signal {
                kind: SignalKind::AgentRequest {
                    need: "a way to read slack".into(),
                    user_request: String::new(),
                    tools: Vec::new(),
                    code_mode: Vec::new(),
                    ranking: Vec::new(),
                },
                ..prompt("")
            },
        )
        .await;

        assert_eq!(ranking(&request)[0], "slack-cli");
    }

    #[tokio::test]
    async fn no_service_or_ranking_off_leaves_the_prompt_unranked() {
        let down = Ranker::new(Some("http://127.0.0.1:9")).unwrap();
        let mut signal = prompt("post to slack");

        down.rank(&mut signal).await;
        assert!(ranking(&signal).is_empty());
        Ranker::new(None).unwrap().rank(&mut signal).await;
        assert!(ranking(&signal).is_empty());
        assert!(Ranker::new(Some("https://embeddings.example.com")).is_err());
    }
}
