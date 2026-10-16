//! Builds the engine from config and skills, rebuilds it when they change,
//! and keeps its memory across restarts. [`crate::scheduler`] runs it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::judge::JudgeConfig;
use crate::scheduler::{EngineHandle, Files, RUN_DEADLINE, Setup, SystemTwoFactory};
use chauffeur_capability_event_gate::{EventGate, EventGateConfig};
use chauffeur_capability_model_router::{ModelRouter, ModelRouterConfig, Provider};
use chauffeur_capability_monitors::{FollowWork, Monitors, MonitorsConfig};
use chauffeur_capability_permission::{Permission, PermissionConfig, load_skills};
use chauffeur_capability_rules::{
    Rulebook, Rules, RulesConfig, Trigger, load_dirs, load_rulebooks, rulebooks_for,
};
use chauffeur_capability_skill_exposure::{SkillExposure, SkillExposureConfig};
use chauffeur_capability_tool_exposure::{ToolExposure, ToolExposureConfig};
use chauffeur_core::{
    Backstop, Capability, Engine, Judging, LearnedShapes, LearningConfig, RedactionConfig,
    Redactor, Reloading, SituationTexts, Watch, load_config,
};
use chauffeur_plugin_anthropic::AnthropicProvider;
use chauffeur_plugin_openai::OpenAiProvider;
use chauffeur_plugin_sourcefed::{Sourcefed, SourcefedConfig};

pub struct EngineOptions {
    pub config_dir: PathBuf,
    /// The skills folder, holding `permission/` contracts and `rules/`.
    pub skills_dir: PathBuf,
    /// Primary System One judge and optional fallback.
    pub judge: JudgeConfig,
    /// Builds a System Two executor for each reasoning worker; `None` leaves
    /// every reasoning unavailable.
    pub system_two: Option<SystemTwoFactory>,
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
    /// Start the engine thread and its judges' workers; resolves once all
    /// are built, or with the first error.
    pub async fn spawn(options: EngineOptions) -> Result<Self, String> {
        let setup = Setup {
            judge: options.judge.clone(),
            system_two: options.system_two.clone(),
            deadline: RUN_DEADLINE,
        };

        Self::launch(
            move || {
                let learned = options
                    .state_file
                    .as_deref()
                    .map(crate::learned::load)
                    .unwrap_or_default();
                let mut engine = build_engine(&options, learned)?;

                if let Some(path) = &options.state_file {
                    restore(&mut engine, path);
                }

                let files = Files {
                    state: options.state_file.clone(),
                    audit: options.audit_file.clone(),
                };

                Ok((reloading(engine, options), files))
            },
            setup,
        )
        .await
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
pub(crate) fn persist(engine: &Engine, path: &Path) {
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
    })
    .with_config(PermissionConfig::load(&config("permission.json"))?);
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
    let situation = SituationTexts::load(&options.config_dir.join("situation.json"))?;

    Ok(Engine::hosted(capabilities)?
        .with_backstop(backstop.with_learned(learned_backstop))
        .with_redactor(redactor)
        .with_learning(learning)
        .with_situation_texts(situation))
}

#[cfg(test)]
mod tests {
    use chauffeur_core::{PipeStep, Plan, Round, RunId, Signal, Situation};

    use super::*;

    struct Named(&'static str);

    impl Capability for Named {
        fn id(&self) -> &str {
            self.0
        }

        fn start(&mut self, _: RunId, _: &Situation, _: &Signal) -> Plan {
            Plan::Skip
        }

        fn resume(&mut self, _: RunId, _: &Signal, _: &Round<'_>, _: usize) -> PipeStep {
            PipeStep::Done(Vec::new())
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
}
