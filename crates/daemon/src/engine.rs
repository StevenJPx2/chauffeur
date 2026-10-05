//! Owns the engine on a dedicated thread. The Jev client blocks on HTTPS, so
//! the engine never runs on the async runtime.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chauffeur_capability_event_gate::{EventGate, EventGateConfig};
use chauffeur_capability_model_router::{ModelRouter, ModelRouterConfig, Provider};
use chauffeur_capability_monitors::{FollowWork, Monitors, MonitorsConfig};
use chauffeur_capability_permission::{Permission, load_skills};
use chauffeur_capability_rules::{
    Rulebook, Rules, RulesConfig, Trigger, load_dirs, load_rulebooks, rulebooks_for,
};
use chauffeur_capability_skill_exposure::{SkillExposure, SkillExposureConfig};
use chauffeur_capability_tool_exposure::{ToolExposure, ToolExposureConfig};
use chauffeur_core::{
    Backstop, Capability, Effect, Engine, Judging, LearnedShapes, LearningConfig, RedactionConfig,
    Redactor, Reloading, Signal, SignalKind, Watch, load_config,
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
    /// Capability IDs to leave out, such as for a benchmark handicap. An ID
    /// no capability has is an error, so a typo cannot pass for a handicap.
    pub disabled: Vec<String>,
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
                let learned = options
                    .state_file
                    .as_deref()
                    .map(crate::learned::load)
                    .unwrap_or_default();
                let mut engine = match build_engine(&options, learned) {
                    Ok(engine) => {
                        let _ = ready.send(Ok(()));
                        engine
                    }
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };

                if let Some(path) = &options.state_file {
                    restore(&mut engine, path);
                }

                let files = Files {
                    state: options.state_file.clone(),
                    audit: options.audit_file.clone(),
                };

                serve(reloading(engine, options), &mut queue, &files);
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

/// Where the engine thread keeps its memory and its decision log.
struct Files {
    state: Option<PathBuf>,
    audit: Option<PathBuf>,
}

/// Run jobs until the daemon shuts down, rebuilding the engine first
/// whenever its config or skills changed.
fn serve(mut engine: Reloading<Engine>, queue: &mut mpsc::Receiver<Job>, files: &Files) {
    let mut pending = VecDeque::new();

    while let Some(job) = next_job(queue, &mut pending) {
        if job.reply.is_closed() {
            continue;
        }
        let engine = engine.current();
        // A model effect only counts when the host is still waiting for it;
        // an abandoned retry must not leave a phantom attempted model or
        // switch-back origin.
        let before =
            matches!(job.signal.kind, SignalKind::ModelError { .. }).then(|| engine.save());
        let result = engine.ingest(&job.signal);

        if job.reply.is_closed() {
            if let Some(before) = before {
                engine.load(before);
            }
            continue;
        }

        if let Some(path) = &files.audit {
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

        if let Some(path) = &files.state {
            persist(engine, path);
            if engine.take_learned_changed() {
                crate::learned::save(engine, path);
            }
        }
    }
}

/// The paths the engine is built from: your config folder and the skills
/// folder's contracts, rules and rulebooks. Handed-over skills under
/// `skills/` are not the daemon's, so they are not watched.
fn watched(options: &EngineOptions) -> Vec<PathBuf> {
    let mut paths = vec![options.config_dir.clone()];

    paths.extend(
        ["permission", "rules", "rulebooks"]
            .iter()
            .map(|folder| options.skills_dir.join(folder)),
    );
    paths
}

/// `engine`, rebuilt from the files whenever they change. The hook carries
/// the running engine's memory and learned patterns into the new one, so
/// sessions, running rulebooks and learned shapes survive an edit.
fn reloading(engine: Engine, options: EngineOptions) -> Reloading<Engine> {
    let watch = Watch::new(watched(&options));

    Reloading::new(
        "config and skills",
        engine,
        watch,
        Box::new(move |old: &Engine| {
            let mut fresh = build_engine(&options, old.learned())?;
            fresh.load(old.save());
            Ok(fresh)
        }),
    )
}

/// The next job to run: a permission request first, since the host holds a
/// tool call until it is answered, then the rest in arrival order. Waits
/// only when nothing is pending; `None` once the daemon is shutting down.
fn next_job(queue: &mut mpsc::Receiver<Job>, pending: &mut VecDeque<Job>) -> Option<Job> {
    if pending.is_empty() {
        pending.push_back(queue.blocking_recv()?);
    }
    while let Ok(job) = queue.try_recv() {
        pending.push_back(job);
    }

    let urgent = pending
        .iter()
        .position(|job| matches!(job.signal.kind, SignalKind::PermissionRequest { .. }))
        .unwrap_or(0);

    pending.remove(urgent)
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
    // The skills folder: `permission/` contracts and `rules/`; then your own
    // `rules/` in the config folder.
    let permission_dir = skills_dir.join("permission");
    let permission = Permission::new(if permission_dir.is_dir() {
        load_skills(&permission_dir)?
    } else {
        Vec::new()
    });
    let mut rules = load_dirs(&[&skills_dir.join("rules"), &config("rules")])?;

    if !idle_reminders {
        rules.retain(|rule| rule.on != Trigger::TurnEnd);
    }

    let rules = Rules::new(rules)
        .with_config(RulesConfig::load(&config("rules.json"))?)
        .with_rulebooks(
            load_rulebooks(&skills_dir.join("rulebooks"))?,
            idle_reminders,
        );
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

/// The shipped rulebooks, from `skills_dir/rulebooks`.
///
/// # Errors
///
/// An unreadable or invalid rulebook.
pub fn shipped_rulebooks(skills_dir: &Path) -> Result<Vec<Rulebook>, String> {
    load_rulebooks(&skills_dir.join("rulebooks"))
}

/// The rulebooks offered in `workspace`, as a host offers them: `shipped`
/// ones in scope, then the project's own.
///
/// # Errors
///
/// An unreadable or invalid project rulebook.
pub fn rulebooks(
    shipped: &[Rulebook],
    workspace: &str,
) -> Result<chauffeur_core::RulebooksResult, String> {
    let books = rulebooks_for(shipped.to_vec(), workspace)?;

    Ok(chauffeur_core::RulebooksResult {
        rulebooks: books
            .into_iter()
            .map(|book| chauffeur_core::RulebookEntry {
                id: book.id,
                name: book.name,
                description: book.description,
                args_required: book.args.required,
            })
            .collect(),
    })
}

/// `capabilities` without the `disabled` IDs, each of which must name one.
fn without(
    capabilities: Vec<Box<dyn Capability>>,
    disabled: &[String],
) -> Result<Vec<Box<dyn Capability>>, String> {
    if let Some(unknown) = disabled.iter().find(|id| {
        !capabilities
            .iter()
            .any(|capability| capability.id() == id.as_str())
    }) {
        let known: Vec<&str> = capabilities
            .iter()
            .map(|capability| capability.id())
            .collect();

        return Err(format!(
            "CHAUFFEUR_DISABLE names {unknown}, which is not one of {}",
            known.join(", ")
        ));
    }

    Ok(capabilities
        .into_iter()
        .filter(|capability| !disabled.iter().any(|id| id == capability.id()))
        .collect())
}

/// Build the engine from the files, with patterns it learned before.
fn build_engine(
    options: &EngineOptions,
    (learned_backstop, learned_shapes): (Vec<String>, LearnedShapes),
) -> Result<Engine, String> {
    let system_one = Box::new(JevClient::new(options.jev.clone())?);
    let capabilities = without(
        capabilities(
            &options.config_dir,
            &options.skills_dir,
            options.idle_reminders,
            options.sourcefed.clone(),
        )?,
        &options.disabled,
    )?;

    let backstop = Backstop::load(&options.config_dir.join("backstop.json"))?;
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

#[cfg(test)]
mod tests {
    use chauffeur_core::{Plan, Situation};

    use super::*;

    struct Named(&'static str);

    impl Capability for Named {
        fn id(&self) -> &str {
            self.0
        }

        fn plan(&mut self, _: &Situation, _: &Signal) -> Plan {
            Plan::Skip
        }

        fn decide(&mut self, _: &Signal, _: Option<&[chauffeur_core::Answer]>) -> Vec<Effect> {
            Vec::new()
        }
    }

    fn all() -> Vec<Box<dyn Capability>> {
        vec![Box::new(Named("rules")), Box::new(Named("skill-exposure"))]
    }

    #[test]
    fn a_disabled_capability_is_left_out() {
        let kept = without(all(), &["skill-exposure".to_string()]).unwrap();
        let ids: Vec<&str> = kept.iter().map(|capability| capability.id()).collect();

        assert_eq!(ids, ["rules"]);
    }

    #[test]
    fn an_unknown_id_is_an_error_naming_the_known_ones() {
        let Err(error) = without(all(), &["skil-exposure".to_string()]) else {
            panic!("expected an error")
        };

        assert!(error.contains("skil-exposure") && error.contains("rules, skill-exposure"));
    }

    fn job(kind: SignalKind) -> Job {
        let (reply, _) = oneshot::channel();

        Job {
            signal: Signal {
                agent_id: "ses".into(),
                at: 1,
                kind,
            },
            reply,
        }
    }

    fn turn_end() -> SignalKind {
        SignalKind::TurnEnd {
            workspace: String::new(),
            subagent: false,
            user_request: String::new(),
            summary: "first".into(),
        }
    }

    fn permission() -> SignalKind {
        SignalKind::PermissionRequest {
            action: "shell".into(),
            resources: Vec::new(),
            request: String::new(),
            workspace: String::new(),
            user_requests: Vec::new(),
            host_decision: chauffeur_core::PermissionDecision::Ask,
        }
    }

    #[test]
    fn a_waiting_permission_request_runs_before_earlier_signals() {
        let (jobs, mut queue) = mpsc::channel(8);
        let mut pending = VecDeque::new();

        for kind in [turn_end(), permission(), turn_end()] {
            jobs.try_send(job(kind)).unwrap();
        }

        let order: Vec<bool> = std::iter::from_fn(|| {
            (!pending.is_empty() || !queue.is_empty())
                .then(|| next_job(&mut queue, &mut pending))
                .flatten()
        })
        .map(|job| matches!(job.signal.kind, SignalKind::PermissionRequest { .. }))
        .collect();

        assert_eq!(order, [true, false, false]);
    }
}
