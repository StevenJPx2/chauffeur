//! One source owner for every host, off the synchronous judgment engine:
//! cheap upstream checks on a timer, and a skill's files fetched when it is attached.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chauffeur_capability_skill_exposure::sources::SourceStore;
use chauffeur_core::{Effect, SkillsResult};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

/// Skills to fetch, and who waits for them.
type Request = (Vec<String>, oneshot::Sender<()>);

/// How long a reply waits for an attached skill's files.
const FETCH_WAIT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(super) struct SkillCatalog {
    current: watch::Receiver<Option<Arc<SkillsResult>>>,
    requests: mpsc::Sender<Request>,
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
    pub(super) fn start(config: PathBuf, cache: PathBuf) -> Result<Self, String> {
        let store = SourceStore::new(config, cache).map_err(|error| error.to_string())?;
        let (publish, current) = watch::channel(None);
        let (stop, stopping) = watch::channel(false);
        // Bounded: a burst of attaches queues at most this many fetches.
        let (requests, received) = mpsc::channel(16);
        let task = tokio::spawn(run(store, publish, stopping, received));

        Ok(Self {
            catalog: SkillCatalog { current, requests },
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

async fn run(
    mut store: SourceStore,
    publish: watch::Sender<Option<Arc<SkillsResult>>>,
    mut stop: watch::Receiver<bool>,
    mut requests: mpsc::Receiver<Request>,
) {
    match store.restore().await {
        Ok(catalog) if !catalog.skills.is_empty() || catalog.sources.is_empty() => {
            publish.send_replace(Some(Arc::new(catalog)));
        }
        Ok(_) => {}
        Err(error) => {
            eprintln!("chauffeur: skill source configuration: {error}");
            publish.send_replace(Some(Arc::new(store.catalog())));
        }
    }

    let mut previous_error = None;

    loop {
        if *stop.borrow() {
            break;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs());

        match store.refresh(now).await {
            Ok(catalog) => {
                previous_error = None;
                publish.send_replace(Some(Arc::new(catalog)));
            }
            Err(error) => {
                let message = error.to_string();

                if previous_error.as_ref() != Some(&message) {
                    eprintln!("chauffeur: skill source configuration: {message}");
                }

                previous_error = Some(message);
                publish.send_replace(Some(Arc::new(store.catalog())));
            }
        }

        tokio::select! {
            _ = stop.changed() => break,
            Some((ids, done)) = requests.recv() => {
                let fetched = tokio::time::timeout(FETCH_WAIT, store.materialize(&ids)).await;

                if let Ok(Err(error)) = fetched {
                    eprintln!("chauffeur: skill files unavailable: {error}");
                }

                let _ = done.send(());
            }
            () = tokio::time::sleep(Duration::from_secs(5)) => {},
        }
    }
}
