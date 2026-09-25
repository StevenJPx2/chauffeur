//! Model router: when a usage limit is hit, System One picks an equivalent
//! model from the same tier, or decides to stay. Later, System One may switch
//! the agent back to the model it left once the limit has likely cleared.

mod provider;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

pub use provider::{Provider, Tier, TierEntry};

use chauffeur_core::{
    Answer, AnswerValue, Capability, ChoiceOption, Effect, ModelRef, Plan, Question, QuestionKind,
    Signal, SignalKind, Situation, load_config,
};
use serde::{Deserialize, Serialize};

pub const ID: &str = "model-router";
pub const STAY: &str = "stay";
const QUESTION: &str = "choice";
const SWITCH_BACK: &str = "switch_back";
/// A switch back is judged no sooner than this after leaving a model.
pub const SWITCH_BACK_AFTER_SECS: u64 = 300;
/// Switch back only when P(better) is at least this, at the usual confidence.
const SWITCH_BACK_AT_OR_ABOVE: f32 = 0.7;
const SWITCH_BACK_MIN_CONFIDENCE: f32 = 0.4;
/// Below this, the judgment is treated as unavailable and the posture applies.
const MIN_CONFIDENCE: f32 = 0.2;
const MAX_TRACKED_AGENTS: usize = 256;
const MAX_PINS: usize = 64;
/// Options offered to System One, after ordering; keeps the choice small.
pub const MAX_CANDIDATES: usize = 8;
const MAX_ERROR_CHARS: usize = 200;

const LIMIT_MESSAGES: &[&str] = &[
    "credit balance",
    "insufficient funds",
    "insufficient account funds",
    "insufficient_quota",
    "quota",
    "rate limit",
    "billing",
    "high demand",
    "overloaded",
    "capacity",
];

const LIMIT_TYPES: &[&str] = &[
    "capacity_exhausted",
    "insufficient_quota",
    "overloaded",
    "overloaded_error",
    "quota",
    "quota_exceeded",
    "rate_limit",
    "rate_limit_error",
    "resource_exhausted",
    "too_many_requests",
    "usage_limit",
];

/// Strict JSON config. `pins` orders preferred fallbacks as `provider/model`.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModelRouterConfig {
    #[serde(default)]
    pub pins: Vec<String>,
}

impl ModelRouterConfig {
    /// Load `path`, or the default config when the file does not exist.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_config::<Self>(path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The config, if within bounds.
    pub fn checked(self) -> Result<Self, String> {
        if self.pins.len() > MAX_PINS {
            return Err(format!("more than {MAX_PINS} pins"));
        }

        Ok(self)
    }
}

#[must_use]
pub fn is_limit_error(error_type: &str, status: Option<u16>, message: &str) -> bool {
    // 503 is a provider out of capacity ("high demand"), which another model avoids.
    if matches!(status, Some(402 | 429 | 503 | 529)) {
        return true;
    }

    let normalized = error_type.trim().to_lowercase().replace([' ', '-'], "_");
    // OpenCode namespaces provider errors, e.g. `provider.quota`.
    let kind = normalized.strip_prefix("provider.").unwrap_or(&normalized);
    // Some providers report an exhausted balance as a plain invalid request.
    let message = message.to_lowercase();

    LIMIT_TYPES.contains(&kind) || LIMIT_MESSAGES.iter().any(|phrase| message.contains(phrase))
}

/// Errors that mean a model cannot serve the agent at all (no access, unknown
/// model), as opposed to a limit that may clear.
const UNUSABLE_TYPES: &[&str] = &[
    "auth",
    "authentication",
    "authentication_error",
    "unauthorized",
    "forbidden",
    "permission",
    "permission_error",
    "not_found",
    "not_found_error",
    "model_not_found",
];

const UNUSABLE_MESSAGES: &[&str] = &[
    "api key",
    "unauthorized",
    "authentication",
    "not authorized",
    "model not found",
    "does not exist",
];

#[must_use]
pub fn is_unusable_error(error_type: &str, status: Option<u16>, message: &str) -> bool {
    if matches!(status, Some(401 | 403 | 404)) {
        return true;
    }

    let normalized = error_type.trim().to_lowercase().replace([' ', '-'], "_");
    let kind = normalized.strip_prefix("provider.").unwrap_or(&normalized);
    let message = message.to_lowercase();

    UNUSABLE_TYPES.contains(&kind)
        || UNUSABLE_MESSAGES
            .iter()
            .any(|phrase| message.contains(phrase))
}

pub struct ModelRouter {
    providers: Vec<Arc<dyn Provider>>,
    pins: Vec<String>,
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
            pins: config.pins,
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
    /// pins first, then other providers, then host order, at most
    /// [`MAX_CANDIDATES`]. Another variant of the current model is never a
    /// candidate: a usage limit applies to the whole model.
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
                tier.is_none() || self.tier(model) == tier || self.pins.contains(&model.key())
            })
            .collect();

        candidates.sort_by_key(|model| {
            let pin = self
                .pins
                .iter()
                .position(|pin| *pin == model.key())
                .unwrap_or(usize::MAX);

            (pin, model.provider == current.provider)
        });
        candidates.truncate(MAX_CANDIDATES);

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

    /// On a user message, ask whether to return to the model the agent left.
    fn plan_switch_back(&mut self, signal: &Signal, current: Option<&ModelRef>) -> Plan {
        let Some(origin) = self.origins.get(&signal.agent_id) else {
            return Plan::Skip;
        };

        // The user changed the model themselves; their choice stands.
        if current.is_some_and(|current| *current != origin.current) {
            self.origins.remove(&signal.agent_id);
            return Plan::Skip;
        }

        let minutes = signal.at.saturating_sub(origin.since) / 60;

        if current.is_none() || signal.at.saturating_sub(origin.since) < SWITCH_BACK_AFTER_SECS {
            return Plan::Skip;
        }

        Plan::Ask(vec![Question {
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
        }])
    }

    fn decide_switch_back(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        let back = answers
            .and_then(|answers| answers.iter().find(|answer| answer.id == SWITCH_BACK))
            .is_some_and(|answer| {
                matches!(answer.value, AnswerValue::Noul(p) if p >= SWITCH_BACK_AT_OR_ABOVE)
                    && answer.effective_confidence() >= SWITCH_BACK_MIN_CONFIDENCE
            });

        if !back {
            return Vec::new();
        }

        let Some(origin) = self.origins.remove(&signal.agent_id) else {
            return Vec::new();
        };

        self.attempted.remove(&signal.agent_id);

        vec![Effect::Model {
            agent_id: signal.agent_id.clone(),
            model: Some(origin.model),
        }]
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

#[derive(Deserialize, Serialize)]
struct Saved {
    attempted: HashMap<String, HashSet<String>>,
    origins: HashMap<String, Origin>,
}

impl Capability for ModelRouter {
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

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        match &signal.kind {
            SignalKind::ModelSucceeded { model } => {
                if self
                    .origins
                    .get(&signal.agent_id)
                    .is_none_or(|origin| origin.current == *model)
                {
                    self.attempted.remove(&signal.agent_id);
                }
                Plan::Skip
            }
            SignalKind::ModelError {
                model,
                error_type,
                status,
                message,
                tool_executed,
                available,
            } => {
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
                    && is_unusable_error(error_type, *status, message);

                if !failed_switch && !is_limit_error(error_type, *status, message) {
                    return Plan::Skip;
                }

                let keep = Effect::Model {
                    agent_id: signal.agent_id.clone(),
                    model: None,
                };

                // A tool already ran in the failed step; retrying elsewhere could repeat it.
                if *tool_executed {
                    return Plan::Settled(vec![keep]);
                }

                self.mark_attempted(&signal.agent_id, model);
                let candidates = self.candidates(&signal.agent_id, model, available);

                if candidates.is_empty() {
                    return Plan::Settled(vec![keep]);
                }

                let error = format!("{error_type}: {}", clip(message, MAX_ERROR_CHARS));

                Plan::Ask(vec![self.question(model, &error, &candidates)])
            }
            SignalKind::UserMessage { model, .. } => {
                let model = model.clone();

                self.plan_switch_back(signal, model.as_ref())
            }
            SignalKind::ToolResult { .. }
            | SignalKind::PermissionRequest { .. }
            | SignalKind::IntegrationEvent { .. }
            | SignalKind::AgentRequest { .. }
            | SignalKind::TurnEnd { .. } => Plan::Skip,
        }
    }

    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        let SignalKind::ModelError {
            model,
            available,
            error_type,
            message,
            ..
        } = &signal.kind
        else {
            return self.decide_switch_back(signal, answers);
        };
        let agent_id = signal.agent_id.clone();
        let candidates = self.candidates(&agent_id, model, available);
        let judged = answers
            .and_then(|answers| answers.iter().find(|answer| answer.id == QUESTION))
            .filter(|answer| answer.effective_confidence() >= MIN_CONFIDENCE);
        let chosen = match judged.map(|answer| &answer.value) {
            Some(AnswerValue::Choice(choice)) if choice == STAY => None,
            Some(AnswerValue::Choice(choice)) => candidates
                .iter()
                .find(|model| model.key() == *choice)
                .cloned(),
            // Posture: stay unblocked on the preferred candidate.
            _ => candidates.first().cloned(),
        };

        match chosen {
            Some(next) => {
                self.mark_attempted(&agent_id, &next);
                self.record_switch(
                    signal,
                    model,
                    &next,
                    format!("{error_type}: {}", clip(message, MAX_ERROR_CHARS)),
                );
                vec![Effect::Model {
                    agent_id,
                    model: Some(next),
                }]
            }
            None => vec![Effect::Model {
                agent_id,
                model: None,
            }],
        }
    }
}

fn clip(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}
