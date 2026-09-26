//! Owns the engine on a dedicated thread. The Jev client blocks on HTTPS, so
//! the engine never runs on the async runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chauffeur_capability_event_gate::{EventGate, EventGateConfig};
use chauffeur_capability_model_router::{ModelRouter, ModelRouterConfig, Provider};
use chauffeur_capability_monitors::{FollowWork, Monitors, MonitorsConfig};
use chauffeur_capability_permission::{Permission, load_skills};
use chauffeur_capability_rules::{Rules, RulesConfig, Trigger, load_dir};
use chauffeur_capability_skill_exposure::{SkillExposure, SkillExposureConfig};
use chauffeur_capability_tool_exposure::{ToolExposure, ToolExposureConfig};
use chauffeur_core::{
    Backstop, Capability, Effect, Engine, Judging, LearningConfig, RedactionConfig, Redactor,
    Signal, load_config,
};
use chauffeur_judge_jev::{JevClient, JevConfig};
use chauffeur_plugin_anthropic::AnthropicProvider;
use chauffeur_plugin_openai::OpenAiProvider;
use chauffeur_plugin_sourcefed::{Sourcefed, SourcefedConfig};
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
    /// The skills folder, holding `permission/` contracts and `rules/`.
    pub skills_dir: PathBuf,
    /// Jev is the System One provider.
    pub jev: JevConfig,
    /// Shipped turn-end rules (idle reminders) are opt-in; tool-result and
    /// project rules always apply.
    pub idle_reminders: bool,
    /// sourcefed's daemon, for following the agent's own work and giving the
    /// event gate each event's monitor; `None` leaves both out.
    pub sourcefed: Option<SourcefedConfig>,
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
                    if job.reply.is_closed() {
                        continue;
                    }
                    // A model effect only counts when the host is still
                    // waiting for it; an abandoned retry must not leave a
                    // phantom attempted model or switch-back origin.
                    let before = matches!(
                        job.signal.kind,
                        chauffeur_core::SignalKind::ModelError { .. }
                    )
                    .then(|| engine.save());
                    let result = engine.ingest(&job.signal);

                    if job.reply.is_closed() {
                        if let Some(before) = before {
                            engine.load(before);
                        }
                        continue;
                    }

                    if let Some(path) = &audit_file {
                        crate::audit::append(path, &job.signal, engine.trace(), &result, |text| {
                            engine.redact_for_audit(text)
                        });
                    }

                    if job.reply.send(result).is_err() {
                        if let Some(before) = before {
                            engine.load(before);
                        }
                        continue;
                    }

                    if let Some(path) = &state_file {
                        persist(&engine, path);
                        if engine.take_learned_changed() {
                            crate::learned::save(&engine, path);
                        }
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

/// Every capability, each with its shipped defaults overlaid by your file of
/// the same name in `config_dir`. Permission answers through its contracts;
/// every other capability is judged.
fn capabilities(
    config_dir: &Path,
    skills_dir: &Path,
    idle_reminders: bool,
    sourcefed: Option<SourcefedConfig>,
) -> Result<Vec<Box<dyn Capability>>, String> {
    let config = |name: &str| config_dir.join(name);
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(AnthropicProvider::load(&config(
            "providers/anthropic.json",
        ))?),
        Arc::new(OpenAiProvider::load(&config("providers/openai.json"))?),
    ];
    let router = ModelRouter::new(
        providers,
        ModelRouterConfig::load(&config("model-router.json"))?,
    );
    let skills = SkillExposure::new(SkillExposureConfig::load(&config("skill-exposure.json"))?);
    let tools = ToolExposure::new(ToolExposureConfig::load(&config("tool-exposure.json"))?);
    // The skills folder: `permission/` contracts and `rules/`.
    let permission_dir = skills_dir.join("permission");
    let permission = Permission::new(if permission_dir.is_dir() {
        load_skills(&permission_dir)?
    } else {
        Vec::new()
    });
    let mut rules = load_dir(&skills_dir.join("rules"))?;

    if !idle_reminders {
        rules.retain(|rule| rule.on != Trigger::TurnEnd);
    }

    let rules = Rules::new(rules).with_config(RulesConfig::load(&config("rules.json"))?);
    let gate = EventGateConfig::load(&config("event-gate.json"))?;
    let mut capabilities: Vec<Box<dyn Capability>> = vec![
        Box::new(Judging::new(router)),
        Box::new(Judging::new(skills)),
        Box::new(Judging::new(tools)),
        Box::new(permission),
        Box::new(Judging::new(rules)),
    ];

    match sourcefed {
        Some(sourcefed) => {
            let monitors: Arc<dyn Monitors> = Arc::new(Sourcefed::new(sourcefed)?);
            let follow = MonitorsConfig::load(&config("monitors.json"))?;

            capabilities.push(Box::new(Judging::new(
                EventGate::with_monitors(Arc::clone(&monitors)).with_config(gate),
            )));
            capabilities.push(Box::new(Judging::new(
                FollowWork::new(monitors).with_config(follow),
            )));
        }
        None => capabilities.push(Box::new(Judging::new(
            EventGate::default().with_config(gate),
        ))),
    }

    Ok(capabilities)
}

fn build_engine(options: EngineOptions) -> Result<Engine, String> {
    let system_one = Box::new(JevClient::new(options.jev)?);
    let capabilities = capabilities(
        &options.config_dir,
        &options.skills_dir,
        options.idle_reminders,
        options.sourcefed,
    )?;

    let backstop = Backstop::load(&options.config_dir.join("backstop.json"))?;
    let (learned_backstop, learned_shapes) = options
        .state_file
        .as_deref()
        .map(crate::learned::load)
        .unwrap_or_default();
    let redaction_path = options.config_dir.join("redaction.json");
    let config: RedactionConfig = load_config(&redaction_path)?;
    let redactor = Redactor::new(config, learned_shapes)
        .map_err(|error| format!("{}: {error}", redaction_path.display()))?;
    let learning = LearningConfig::load(&options.config_dir.join("learning.json"))?;

    Ok(Engine::new(system_one, capabilities)?
        .with_backstop(backstop.with_learned(learned_backstop))
        .with_redactor(redactor)
        .with_learning(learning))
}
