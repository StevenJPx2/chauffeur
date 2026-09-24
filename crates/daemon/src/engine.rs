//! Owns the engine on a dedicated thread. The Jev client blocks on HTTPS, so
//! the engine never runs on the async runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chauffeur_capability_event_gate::EventGate;
use chauffeur_capability_idle_reminder::{IdleReminder, Plugin, compose};
use chauffeur_capability_model_router::{ModelRouter, ModelRouterConfig, Provider};
use chauffeur_capability_permission::{Permission, load_skills};
use chauffeur_capability_skill_exposure::SkillExposure;
use chauffeur_capability_tool_exposure::{ToolExposure, ToolExposureConfig};
use chauffeur_capability_tool_misuse::{ToolMisuse, load_contracts};
use chauffeur_core::{Backstop, Capability, Effect, Engine, Signal};
use chauffeur_judge_jev::{JevClient, JevConfig};
use chauffeur_plugin_anthropic::AnthropicProvider;
use chauffeur_plugin_git::GitPlugin;
use chauffeur_plugin_github::GitHubPlugin;
use chauffeur_plugin_jira::JiraPlugin;
use chauffeur_plugin_openai::OpenAiProvider;
use tokio::sync::{mpsc, oneshot};

const QUEUE_DEPTH: usize = 64;
const REPLY_TIMEOUT: Duration = Duration::from_secs(15);

struct Job {
    signal: Signal,
    reply: oneshot::Sender<Result<Vec<Effect>, String>>,
}

#[derive(Clone)]
pub struct EngineHandle {
    jobs: mpsc::Sender<Job>,
}

pub struct EngineOptions {
    pub config_dir: PathBuf,
    /// The skills folder, holding `permission/` and `misuse/`
    /// contracts.
    pub skills_dir: PathBuf,
    /// Jev is the System One provider.
    pub jev: JevConfig,
    /// Idle reminders are opt-in; turn ends still reach skill exposure.
    pub idle_reminders: bool,
    /// Where per-agent memory is kept across restarts; `None` keeps it in
    /// memory only.
    pub state_file: Option<PathBuf>,
    /// Where every decision is appended as JSONL; `None` keeps no log.
    pub audit_file: Option<PathBuf>,
}

impl EngineHandle {
    /// Start the engine thread; resolves once the engine is built or failed.
    pub async fn spawn(options: EngineOptions) -> Result<Self, String> {
        let (jobs, mut queue) = mpsc::channel::<Job>(QUEUE_DEPTH);
        let (ready, started) = oneshot::channel();

        std::thread::Builder::new()
            .name("chauffeur-engine".into())
            .spawn(move || {
                let state_file = options.state_file.clone();
                let audit_file = options.audit_file.clone();
                let mut engine = match build_engine(options) {
                    Ok(engine) => {
                        let _ = ready.send(Ok(()));
                        engine
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };

                if let Some(path) = &state_file {
                    restore(&mut engine, path);
                }

                while let Some(job) = queue.blocking_recv() {
                    let result = engine.ingest(&job.signal);

                    if let Some(path) = &audit_file {
                        crate::audit::append(path, &job.signal, engine.trace(), &result);
                    }

                    let _ = job.reply.send(result);

                    if let Some(path) = &state_file {
                        persist(&engine, path);
                    }
                }
            })
            .map_err(|error| format!("spawn engine thread: {error}"))?;

        started
            .await
            .map_err(|_| "engine thread exited during startup".to_string())??;

        Ok(Self { jobs })
    }

    pub async fn ingest(&self, signal: Signal) -> Result<Vec<Effect>, String> {
        let (reply, result) = oneshot::channel();

        self.jobs
            .try_send(Job { signal, reply })
            .map_err(|error| format!("engine unavailable: {error}"))?;

        tokio::time::timeout(REPLY_TIMEOUT, result)
            .await
            .map_err(|_| "engine timed out".to_string())?
            .map_err(|_| "engine dropped the signal".to_string())?
    }
}

/// Larger state files are ignored rather than read.
const MAX_STATE_BYTES: u64 = 8 * 1024 * 1024;

fn restore(engine: &mut Engine, path: &Path) {
    let Ok(metadata) = std::fs::metadata(path) else {
        return;
    };

    if metadata.len() > MAX_STATE_BYTES {
        eprintln!(
            "chauffeur: {} exceeds {MAX_STATE_BYTES} bytes; starting fresh",
            path.display()
        );
        return;
    }

    match std::fs::read(path).map(|bytes| serde_json::from_slice(&bytes)) {
        Ok(Ok(state)) => engine.load(state),
        Ok(Err(error)) => eprintln!(
            "chauffeur: {} is not valid state ({error}); starting fresh",
            path.display()
        ),
        Err(error) => eprintln!(
            "chauffeur: read {}: {error}; starting fresh",
            path.display()
        ),
    }
}

/// Write atomically: a crash mid-write leaves the previous state intact.
fn persist(engine: &Engine, path: &Path) {
    let temporary = path.with_extension("json.tmp");
    let result = serde_json::to_vec(&engine.save())
        .map_err(|error| error.to_string())
        .and_then(|bytes| std::fs::write(&temporary, bytes).map_err(|error| error.to_string()))
        .and_then(|()| std::fs::rename(&temporary, path).map_err(|error| error.to_string()));

    if let Err(error) = result {
        eprintln!("chauffeur: state not saved to {}: {error}", path.display());
    }
}

fn build_engine(options: EngineOptions) -> Result<Engine, String> {
    let system_one = Box::new(JevClient::new(options.jev)?);
    let providers: Vec<Arc<dyn Provider>> =
        vec![Arc::new(AnthropicProvider), Arc::new(OpenAiProvider)];
    let router = ModelRouter::new(
        providers,
        ModelRouterConfig::load(&options.config_dir.join("model-router.json"))?,
    );
    let tools = ToolExposure::new(ToolExposureConfig::load(
        &options.config_dir.join("tool-exposure.json"),
    )?);
    // The skills folder: `permission/` and `misuse/` contracts.
    let permission_dir = options.skills_dir.join("permission");
    let permission = Permission::new(if permission_dir.is_dir() {
        load_skills(&permission_dir)?
    } else {
        Vec::new()
    });
    let plugins: Vec<Box<dyn Plugin>> = vec![
        Box::new(GitHubPlugin),
        Box::new(JiraPlugin),
        Box::new(GitPlugin),
    ];
    let idle = options
        .idle_reminders
        .then(|| compose(&plugins).map(IdleReminder::new))
        .transpose()?;
    let misuse = ToolMisuse::new(load_contracts(&options.skills_dir.join("misuse"))?);
    let mut capabilities: Vec<Box<dyn Capability>> = vec![
        Box::new(router),
        Box::new(SkillExposure::default()),
        Box::new(tools),
        Box::new(permission),
        Box::new(misuse),
        Box::new(EventGate),
    ];

    if let Some(idle) = idle {
        capabilities.push(Box::new(idle));
    }

    let backstop = Backstop::load(&options.config_dir.join("backstop.json"))?;

    Ok(Engine::new(system_one, capabilities)?.with_backstop(backstop))
}
