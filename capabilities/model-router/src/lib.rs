//! Model router: when a usage limit is hit, System One picks an equivalent
//! model from the same tier, or decides to stay. Later, System One may switch
//! the agent back to the model it left once the limit has likely cleared.
//! Run it as `Judging::new(ModelRouter::new(providers, config))`.

mod config;
mod provider;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub use config::{ErrorWords, MAX_CANDIDATES, ModelRouterConfig, SwitchBack};
pub use provider::{MAX_TIER_ROWS, Provider, Tier, TierEntry, TierTable};

use chauffeur_core::judge::strategy;
use chauffeur_core::{
    ChoiceOption, Effect, Judge, Judged, ModelRef, Question, QuestionKind, Signal, SignalKind,
    Situation,
};
use serde::{Deserialize, Serialize};

pub const ID: &str = "model-router";
pub const STAY: &str = "stay";
const QUESTION: &str = "choice";
const SWITCH_BACK: &str = "switch_back";
const MAX_TRACKED_AGENTS: usize = 256;
const MAX_ERROR_CHARS: usize = 200;

pub struct ModelRouter {
    providers: Vec<Arc<dyn Provider>>,
    config: ModelRouterConfig,
    attempted: HashMap<String, HashSet<String>>,
    origins: HashMap<String, Origin>,
}

/// The model an agent left on a usage limit, and where it went.
#[derive(Clone, Deserialize, Serialize)]
struct Origin {
    model: ModelRef,
    current: ModelRef,
    since: u64,
    error: String,
}

impl ModelRouter {
    #[must_use]
    pub fn new(providers: Vec<Arc<dyn Provider>>, config: ModelRouterConfig) -> Self {
        Self {
            providers,
            config,
            attempted: HashMap::new(),
            origins: HashMap::new(),
        }
    }

    fn tier(&self, model: &ModelRef) -> Option<Tier> {
        self.provider(&model.provider)
            .and_then(|provider| provider.tier(&model.model, model.variant.as_deref()))
    }

    fn provider(&self, id: &str) -> Option<&Arc<dyn Provider>> {
        self.providers.iter().find(|provider| provider.id() == id)
    }

    /// Each usable host model at every variant its tier table names; a model
    /// without a table row keeps its default variant.
    fn expanded(&self, available: &[chauffeur_core::AvailableModel]) -> Vec<ModelRef> {
        available
            .iter()
            .filter(|entry| entry.usable)
            .flat_map(|entry| {
                let variants = self
                    .provider(&entry.model.provider)
                    .map(|provider| provider.variants(&entry.model.model))
                    .filter(|variants| !variants.is_empty())
                    .unwrap_or_else(|| vec![None]);

                variants.into_iter().map(|variant| ModelRef {
                    variant: variant.map(str::to_string),
                    ..entry.model.clone()
                })
            })
            .collect()
    }

    fn mark_attempted(&mut self, agent_id: &str, model: &ModelRef) {
        if !self.attempted.contains_key(agent_id) && self.attempted.len() >= MAX_TRACKED_AGENTS {
            self.attempted.clear();
        }

        self.attempted
            .entry(agent_id.to_string())
            .or_default()
            .insert(model.key());
    }

    /// Usable same-tier models (at a thinking variant) not yet tried, ordered
    /// pins first, then other providers, then host order, at most the
    /// configured `max_candidates`. Another variant of the current model is
    /// never a candidate: a usage limit applies to the whole model.
    fn candidates(
        &self,
        agent_id: &str,
        current: &ModelRef,
        available: &[chauffeur_core::AvailableModel],
    ) -> Vec<ModelRef> {
        let tried = self.attempted.get(agent_id);
        let tier = self.tier(current);
        let mut candidates: Vec<ModelRef> = self
            .expanded(available)
            .into_iter()
            .filter(|model| !(model.provider == current.provider && model.model == current.model))
            .filter(|model| tried.is_none_or(|tried| !tried.contains(&model.key())))
            // A pinned model is a declared fallback, so tiers do not constrain it.
            .filter(|model| {
                tier.is_none()
                    || self.tier(model) == tier
                    || self.config.pins.contains(&model.key())
            })
            .collect();

        candidates.sort_by_key(|model| {
            let pin = self
                .config
                .pins
                .iter()
                .position(|pin| *pin == model.key())
                .unwrap_or(usize::MAX);

            (pin, model.provider == current.provider)
        });
        candidates.truncate(self.config.max_candidates);

        candidates
    }

    /// Remember the model left behind; a chain of switches keeps the first.
    fn record_switch(&mut self, signal: &Signal, left: &ModelRef, next: &ModelRef, error: String) {
        if !self.origins.contains_key(&signal.agent_id) && self.origins.len() >= MAX_TRACKED_AGENTS
        {
            self.origins.clear();
        }

        let origin = self
            .origins
            .entry(signal.agent_id.clone())
            .or_insert_with(|| Origin {
                model: left.clone(),
                current: next.clone(),
                since: signal.at,
                error: error.clone(),
            });

        // The first limit is what switching back waits on.
        origin.current = next.clone();
    }

    /// On a user message, judge whether to return to the model the agent
    /// left.
    fn switch_back(
        &mut self,
        signal: &Signal,
        current: Option<&ModelRef>,
    ) -> Option<Judge<Verdict>> {
        let origin = self.origins.get(&signal.agent_id)?;

        // The user changed the model themselves; their choice stands.
        if current.is_some_and(|current| *current != origin.current) {
            self.origins.remove(&signal.agent_id);
            return None;
        }

        let minutes = signal.at.saturating_sub(origin.since) / 60;

        if current.is_none()
            || signal.at.saturating_sub(origin.since) < self.config.switch_back.after_seconds
        {
            return None;
        }

        let question = Question {
            id: SWITCH_BACK.into(),
            // Asked as a fact about the limit: Jev judges that far better than
            // a trade-off against the cache cost.
            instructions: format!(
                "About {minutes} minutes ago the coding agent's model {} failed with this usage \
                 limit: {}. Has that limit most likely cleared by now? Short per-minute rate \
                 limits and temporary overloads clear within minutes; limits that reset in hours, \
                 and exhausted quotas or balances, have not cleared.",
                origin.model.key(),
                origin.error,
            ),
            kind: QuestionKind::Noul,
        };

        let bar = self.config.switch_back.bar.yes();

        Some(strategy::single(question, bar).map(Verdict::SwitchBack))
    }

    /// Return to the model left behind, forgetting the failover.
    fn return_to_origin(&mut self, agent_id: &str) -> Vec<Effect> {
        let Some(origin) = self.origins.remove(agent_id) else {
            return Vec::new();
        };

        self.attempted.remove(agent_id);

        vec![Effect::Model {
            agent_id: agent_id.to_string(),
            model: Some(origin.model),
        }]
    }

    /// On a model error, the failover judgment, or `None` when the error is
    /// not the router's to handle.
    fn failover(&mut self, signal: &Signal) -> Option<Judge<Verdict>> {
        let SignalKind::ModelError {
            model,
            error_type,
            status,
            message,
            tool_executed,
            available,
        } = &signal.kind
        else {
            return None;
        };

        // A retry is still running on the origin: a proposed switch
        // never reached the host (for example, it was cancelled).
        // Let the next retry choose that fallback again.
        if self
            .origins
            .get(&signal.agent_id)
            .is_some_and(|origin| origin.model == *model && origin.current != *model)
        {
            self.origins.remove(&signal.agent_id);
            self.attempted.remove(&signal.agent_id);
        }
        // A model the router switched to that cannot serve the agent is a
        // failed switch: move on to the next candidate.
        let failed_switch = self
            .origins
            .get(&signal.agent_id)
            .is_some_and(|origin| origin.current == *model)
            && self.config.is_unusable_error(error_type, *status, message);

        if !failed_switch && !self.config.is_limit_error(error_type, *status, message) {
            return None;
        }

        // A tool already ran in the failed step; retrying elsewhere could repeat it.
        if *tool_executed {
            return Some(Judge::done(Verdict::Keep));
        }

        self.mark_attempted(&signal.agent_id, model);
        let candidates = self.candidates(&signal.agent_id, model, available);

        if candidates.is_empty() {
            return Some(Judge::done(Verdict::Keep));
        }

        let error = format!("{error_type}: {}", clip(message, MAX_ERROR_CHARS));
        let question = self.question(model, &error, &candidates);
        let from = model.clone();
        let rule = self.config.pick_confidence.pick();

        Some(Judge::ask(question, move |answer| {
            pick(rule.chosen(answer), candidates, from, error)
        }))
    }

    fn question(&self, current: &ModelRef, error: &str, candidates: &[ModelRef]) -> Question {
        let mut options: Vec<ChoiceOption> = candidates
            .iter()
            .map(|model| ChoiceOption {
                value: model.key(),
                description: format!(
                    "Switch to {} ({} tier, provider {}).",
                    model.key(),
                    self.tier(model).map_or("unknown", Tier::label),
                    model.provider
                ),
            })
            .collect();

        options.push(ChoiceOption {
            value: STAY.into(),
            description: format!("Keep {} and wait for the limit to clear.", current.key()),
        });

        Question {
            id: QUESTION.into(),
            instructions: format!(
                "The coding agent's model {} just failed with a usage limit ({error}). \
                 Switching discards the prompt cache on the new provider. Choose the model \
                 that best fits the agent's recent work. Choose stay only if this limit will \
                 clear on its own soon, such as a short rate limit; an exhausted quota or \
                 balance will not clear by waiting.",
                current.key()
            ),
            kind: QuestionKind::Choice { options },
        }
    }
}

/// What a finished judgment asks the router to do.
pub enum Verdict {
    /// Keep the current model: stay, or failover is not possible.
    Keep,
    /// Leave `from`, which failed with `error`, for `to`.
    Switch {
        from: ModelRef,
        to: ModelRef,
        error: String,
    },
    /// Whether to return to the model the agent left.
    SwitchBack(bool),
}

/// The model a failover judgment picked. A failed or unsure judgment falls
/// back on the posture: stay unblocked on the preferred candidate.
fn pick(
    chosen: Option<String>,
    candidates: Vec<ModelRef>,
    from: ModelRef,
    error: String,
) -> Verdict {
    let to = match chosen.as_deref() {
        Some(STAY) => None,
        Some(choice) => candidates.into_iter().find(|model| model.key() == choice),
        None => candidates.into_iter().next(),
    };

    to.map_or(Verdict::Keep, |to| Verdict::Switch { from, to, error })
}

#[derive(Deserialize, Serialize)]
struct Saved {
    attempted: HashMap<String, HashSet<String>>,
    origins: HashMap<String, Origin>,
}

impl Judged for ModelRouter {
    type Verdict = Verdict;

    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        serde_json::to_value(Saved {
            attempted: self.attempted.clone(),
            origins: self.origins.clone(),
        })
        .ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        if let Ok(saved) = serde_json::from_value::<Saved>(state) {
            self.attempted = saved
                .attempted
                .into_iter()
                .take(MAX_TRACKED_AGENTS)
                .collect();
            self.origins = saved.origins.into_iter().take(MAX_TRACKED_AGENTS).collect();
        }
    }

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<Verdict>> {
        match &signal.kind {
            SignalKind::ModelSucceeded { model } => {
                if self
                    .origins
                    .get(&signal.agent_id)
                    .is_none_or(|origin| origin.current == *model)
                {
                    self.attempted.remove(&signal.agent_id);
                }
                None
            }
            SignalKind::ModelError { .. } => self.failover(signal),
            SignalKind::UserMessage { model, .. } => {
                let model = model.clone();

                self.switch_back(signal, model.as_ref())
            }
            SignalKind::ToolResult { .. }
            | SignalKind::PermissionRequest { .. }
            | SignalKind::IntegrationEvent { .. }
            | SignalKind::AgentRequest { .. }
            | SignalKind::TurnEnd { .. } => None,
        }
    }

    fn act(&mut self, signal: &Signal, verdict: Verdict) -> Vec<Effect> {
        let agent_id = signal.agent_id.clone();

        match verdict {
            Verdict::Keep => vec![Effect::Model {
                agent_id,
                model: None,
            }],
            Verdict::Switch { from, to, error } => {
                self.mark_attempted(&agent_id, &to);
                self.record_switch(signal, &from, &to, error);
                vec![Effect::Model {
                    agent_id,
                    model: Some(to),
                }]
            }
            Verdict::SwitchBack(true) => self.return_to_origin(&agent_id),
            Verdict::SwitchBack(false) => Vec::new(),
        }
    }
}

fn clip(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}
