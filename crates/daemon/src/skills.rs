//! One source owner for every host, off the synchronous judgment engine:
//! cheap upstream checks on a timer, a skill's files fetched when it is
//! attached, and each new upstream version judged before agents see it.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chauffeur_capability_skill_exposure::sources::{Review, ReviewJob, SourceStore, read_verdict};
use chauffeur_core::{Effect, SkillAcceptResult, SkillsResult};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::{JoinHandle, JoinSet};

use crate::scheduler::EngineHandle;

/// Skills to fetch, and who waits for them.
type Request = (Vec<String>, oneshot::Sender<()>);
/// A skill to accept, and who waits for the answer.
type Accept = (String, oneshot::Sender<Result<SkillAcceptResult, String>>);

/// How long a reply waits for an attached skill's files.
const FETCH_WAIT: Duration = Duration::from_secs(10);
/// Reviews at once: System Two has two workers, and agents come first.
const CONCURRENT_REVIEWS: usize = 2;
/// A review's budget: the engine's whole run limit, plus its reply reserve.
const REVIEW_DEADLINE_MS: u64 = 14_150;

#[derive(Clone)]
pub(super) struct SkillCatalog {
    current: watch::Receiver<Option<Arc<SkillsResult>>>,
    requests: mpsc::Sender<Request>,
    accepts: mpsc::Sender<Accept>,
}

impl SkillCatalog {
    pub(super) async fn current(&self) -> Result<Arc<SkillsResult>, String> {
        let mut receiver = self.current.clone();
        let available =
            tokio::time::timeout(Duration::from_secs(25), receiver.wait_for(Option::is_some))
                .await
                .map_err(|_| "skill catalog initialization timed out")?
                .map_err(|_| "skill source worker stopped")?;

        available
            .as_ref()
            .cloned()
            .ok_or_else(|| "skill catalog unavailable".into())
    }

    /// Approve a remote skill's latest upstream version.
    pub(super) async fn accept(&self, id: String) -> Result<SkillAcceptResult, String> {
        let (reply, answer) = oneshot::channel();

        self.accepts
            .send((id, reply))
            .await
            .map_err(|_| "skill source worker stopped")?;

        tokio::time::timeout(Duration::from_secs(25), answer)
            .await
            .map_err(|_| "skill source worker is busy")?
            .map_err(|_| "skill source worker stopped")?
    }

    /// Fetch the files of every remote skill these effects attach, waiting at
    /// most [`FETCH_WAIT`]. Local skills and other effects pass untouched; a
    /// failed or slow fetch still delivers the skill's instructions.
    pub(super) async fn prepare(&self, effects: &[Effect]) {
        let ids: Vec<String> = effects
            .iter()
            .flat_map(|effect| match effect {
                Effect::Context { skills, .. } => skills.clone(),
                _ => Vec::new(),
            })
            .collect();

        if ids.is_empty() {
            return;
        }

        let (done, waiting) = oneshot::channel();
        let sent = self.requests.try_send((ids, done));

        if sent.is_ok() {
            let _ = tokio::time::timeout(FETCH_WAIT, waiting).await;
        }
    }
}

/// Scope owner: normal shutdown joins the bounded worker; early exits cancel its process work.
pub(super) struct SkillWorker {
    pub(super) catalog: SkillCatalog,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
}

impl SkillWorker {
    /// The worker, judging new upstream versions through `engine`.
    pub(super) fn start(
        config: PathBuf,
        cache: PathBuf,
        engine: EngineHandle,
    ) -> Result<Self, String> {
        let store = SourceStore::new(config, cache)
            .map_err(|error| error.to_string())?
            .with_reviews();
        let (publish, current) = watch::channel(None);
        let (stop, stopping) = watch::channel(false);
        // Bounded: a burst of attaches queues at most this many fetches.
        let (requests, received) = mpsc::channel(16);
        let (accepts, accepted) = mpsc::channel(4);
        let task = tokio::spawn(run(
            Worker {
                store,
                publish,
                engine,
                reviews: JoinSet::new(),
                busy: HashSet::new(),
            },
            stopping,
            received,
            accepted,
        ));

        Ok(Self {
            catalog: SkillCatalog {
                current,
                requests,
                accepts,
            },
            stop,
            task: Some(task),
        })
    }

    pub(super) async fn finish(mut self) -> Result<(), String> {
        let _ = self.stop.send(true);

        if let Some(task) = self.task.take() {
            task.await
                .map_err(|error| format!("join skill source worker: {error}"))?;
        }

        Ok(())
    }
}

impl Drop for SkillWorker {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

struct Worker {
    store: SourceStore,
    publish: watch::Sender<Option<Arc<SkillsResult>>>,
    engine: EngineHandle,
    /// Reviews in flight, each answering with its job and verdict.
    reviews: JoinSet<(ReviewJob, Review)>,
    /// The source and skill of each review in flight.
    busy: HashSet<(String, String)>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

impl Worker {
    fn publish(&self) {
        self.publish
            .send_replace(Some(Arc::new(self.store.catalog())));
    }

    /// Start the reviews that are due, up to [`CONCURRENT_REVIEWS`] in flight.
    async fn start_reviews(&mut self, now: u64) {
        let room = CONCURRENT_REVIEWS.saturating_sub(self.reviews.len());

        for job in self
            .store
            .due_reviews(now, &self.busy)
            .into_iter()
            .take(room)
        {
            let request = tokio::time::timeout(FETCH_WAIT, self.store.review_request(&job)).await;
            let request = match request {
                Ok(Ok(request)) => request,
                Ok(Err(error)) => {
                    self.finish(
                        &job,
                        &Review::undetermined(format!("its files: {error}")),
                        now,
                    )
                    .await;
                    continue;
                }
                Err(_) => {
                    self.finish(&job, &Review::undetermined("its files took too long"), now)
                        .await;
                    continue;
                }
            };
            let engine = self.engine.clone();

            self.busy.insert((job.source.clone(), job.skill.clone()));
            self.reviews.spawn(async move {
                let review = match engine
                    .ingest_with_deadline(request.signal(now), Some(REVIEW_DEADLINE_MS))
                    .await
                {
                    Ok(effects) => read_verdict(&effects),
                    Err(error) => Review::undetermined(format!("the engine failed: {error}")),
                };

                (job, review)
            });
        }
    }

    async fn finish(&mut self, job: &ReviewJob, review: &Review, now: u64) {
        self.busy.remove(&(job.source.clone(), job.skill.clone()));

        if let Err(error) = self.store.record(job, review, now).await {
            eprintln!("chauffeur: skill review of {}: {error}", job.skill);
        }

        self.publish();
    }
}

async fn run(
    mut worker: Worker,
    mut stop: watch::Receiver<bool>,
    mut requests: mpsc::Receiver<Request>,
    mut accepts: mpsc::Receiver<Accept>,
) {
    match worker.store.restore().await {
        Ok(catalog) if !catalog.skills.is_empty() || catalog.sources.is_empty() => {
            worker.publish.send_replace(Some(Arc::new(catalog)));
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!("chauffeur: skill source configuration: {error}");
            worker.publish();
        }
    }

    let mut previous_error = None;

    loop {
        if *stop.borrow() {
            break;
        }

        let now = now();

        match worker.store.refresh(now).await {
            Ok(catalog) => {
                previous_error = None;
                worker.publish.send_replace(Some(Arc::new(catalog)));
            }
            Err(error) => {
                let message = error.to_string();

                if previous_error.as_ref() != Some(&message) {
                    eprintln!("chauffeur: skill source configuration: {message}");
                }

                previous_error = Some(message);
                worker.publish();
            }
        }

        worker.start_reviews(now).await;

        tokio::select! {
            _ = stop.changed() => break,
            Some(done) = worker.reviews.join_next(), if !worker.reviews.is_empty() => {
                match done {
                    Ok((job, review)) => worker.finish(&job, &review, crate::skills::now()).await,
                    // A review task that died leaves no job to clear: start afresh.
                    Err(_) if worker.reviews.is_empty() => worker.busy.clear(),
                    Err(_) => {}
                }
            }
            Some((id, reply)) = accepts.recv() => {
                let accepted = worker.store.accept(&id).await.map_err(|error| error.to_string());

                worker.publish();
                let _ = reply.send(accepted);
            }
            Some((ids, done)) = requests.recv() => {
                let fetched = tokio::time::timeout(FETCH_WAIT, worker.store.materialize(&ids)).await;

                if let Ok(Err(error)) = fetched {
                    eprintln!("chauffeur: skill files unavailable: {error}");
                }

                let _ = done.send(());
            }
            () = tokio::time::sleep(Duration::from_secs(5)) => {},
        }
    }

    worker.reviews.abort_all();
}
