//! Skill exposure: on each user message, and when the agent asks for help
//! itself, System One judges every offered skill in one call and each one the
//! request needs is attached. After tool results and at turn end it picks at
//! most one hand-over, in case the agent lost track, such as driving a
//! browser where a dedicated skill exists. Run it as
//! `Judging::new(SkillExposure::new(config))`, or with the shipped config as
//! `Judging::new(SkillExposure::default())`.

mod config;
mod fanout;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use chauffeur_core::judge::strategy;
use chauffeur_core::{
    CatalogEntry, ChoiceOption, Delivery, Effect, Judge, Judged, Question, QuestionKind, Signal,
    SignalKind, Situation,
};

pub use config::{Budget, Drift, SkillExposureConfig};

pub const ID: &str = "skill-exposure";
/// The drift option that hands over nothing.
pub const NONE: &str = chauffeur_core::judge::NONE;
/// Skills judged per signal; the catalog is truncated in host order.
pub const MAX_OPTIONS: usize = 64;
const MAX_AGENTS: usize = 256;
const DRIFT: &str = "drift";
/// A shell command naming this CLI drives a browser.
const BROWSER_CLI: &str = "browser-harness";

/// What one agent can still be offered, from its latest user message.
#[derive(Clone, Default, Deserialize, Serialize)]
struct Agent {
    skills: Vec<CatalogEntry>,
    attached: HashSet<String>,
    last_drift_at: Option<u64>,
    #[serde(default)]
    tools_seen: u32,
    #[serde(default)]
    drift_seen: u32,
    #[serde(default)]
    last_tool: Option<String>,
    #[serde(default)]
    browser_tool: bool,
}

impl Agent {
    fn offerable(&self) -> Vec<&CatalogEntry> {
        let mut seen = HashSet::new();

        self.skills
            .iter()
            .filter(|skill| {
                skill.id != NONE
                    && !self.attached.contains(&skill.id)
                    && seen.insert(skill.id.as_str())
            })
            .take(MAX_OPTIONS)
            .collect()
    }

    fn drift_due(&self, at: u64, cooldown_seconds: u64) -> bool {
        self.last_drift_at
            .is_none_or(|last| at.saturating_sub(last) >= cooldown_seconds)
    }

    /// Remember the latest tool action and whether it drove a browser, so a
    /// browser hand-over is offered only for actual browser use.
    fn record_tool(&mut self, tool: &str, ok: bool, input: &str) {
        self.tools_seen = self.tools_seen.saturating_add(1);
        self.last_tool = Some(format!("tool: {tool}; succeeded: {ok}; input: {input}"));
        self.browser_tool = tool.starts_with("browser")
            || (matches!(tool, "shell" | "execute")
                && (input.contains(BROWSER_CLI) || input.contains("tools.browser.")));
    }
}

/// `default()` uses the shipped config.
#[derive(Default)]
pub struct SkillExposure {
    config: SkillExposureConfig,
    agents: HashMap<String, Agent>,
}

impl SkillExposure {
    #[must_use]
    pub fn new(config: SkillExposureConfig) -> Self {
        Self {
            config,
            agents: HashMap::new(),
        }
    }

    fn agent(&mut self, agent_id: &str) -> &mut Agent {
        if !self.agents.contains_key(agent_id) && self.agents.len() >= MAX_AGENTS {
            self.agents.clear();
        }

        self.agents.entry(agent_id.to_string()).or_default()
    }

    /// Every offered skill, judged in one round.
    fn fan_out(&self, signal: &Signal) -> Option<Judge<Vec<String>>> {
        let agent = self.agents.get(&signal.agent_id)?;
        let candidates = fanout::candidates(signal, &agent.offerable(), &self.config);

        (!candidates.is_empty()).then(|| strategy::fan_out(candidates))
    }

    /// At a tool result or turn end, whether a drift check is due; records
    /// the tool either way.
    fn drift_due(&mut self, signal: &Signal) -> bool {
        let cooldown_seconds = self.config.drift.cooldown_seconds;
        let Some(agent) = self.agents.get_mut(&signal.agent_id) else {
            return false;
        };

        if let SignalKind::ToolResult {
            tool, ok, input, ..
        } = &signal.kind
        {
            agent.record_tool(tool, *ok, input);
        }

        let turn_end = matches!(signal.kind, SignalKind::TurnEnd { .. });

        if agent.tools_seen == agent.drift_seen
            || (!turn_end && !agent.drift_due(signal.at, cooldown_seconds))
        {
            return false;
        }

        agent.last_drift_at = Some(signal.at);
        agent.drift_seen = agent.tools_seen;

        true
    }

    /// At most one hand-over; a browser-only one after actual browser use.
    fn drift(&self, signal: &Signal) -> Option<Judge<Vec<String>>> {
        let agent = self.agents.get(&signal.agent_id)?;
        let drift = &self.config.drift;
        let offerable: Vec<_> = agent
            .offerable()
            .into_iter()
            .filter(|skill| agent.browser_tool || !drift.browser_only.contains(&skill.id))
            .collect();
        let pick = drift.confidence.pick();

        if offerable.is_empty() {
            return None;
        }

        let instructions = format!(
            "{DRIFT_INSTRUCTIONS} Latest tool action: {}",
            agent.last_tool.as_deref().unwrap_or("unknown")
        );

        Some(
            Judge::ask(choice(DRIFT, &instructions, &offerable), move |answer| {
                pick.chosen(answer)
            })
            .map(|skill| skill.into_iter().collect()),
        )
    }
}

impl Judged for SkillExposure {
    type Verdict = Vec<String>;

    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        serde_json::to_value(&self.agents).ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        if let Ok(agents) = serde_json::from_value::<HashMap<String, Agent>>(state) {
            self.agents = agents.into_iter().take(MAX_AGENTS).collect();
        }
    }

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<Vec<String>>> {
        match &signal.kind {
            SignalKind::UserMessage { skills, .. } => {
                // The host lists only skills not yet attached in this context.
                *self.agent(&signal.agent_id) = Agent {
                    skills: skills.clone(),
                    ..Agent::default()
                };

                self.fan_out(signal)
            }
            SignalKind::AgentRequest { .. } => self.fan_out(signal),
            SignalKind::ToolResult { .. } | SignalKind::TurnEnd { .. }
                if self.drift_due(signal) =>
            {
                self.drift(signal)
            }
            _ => None,
        }
    }

    /// Attach the admitted skills in order while they fit the signal's byte
    /// budget and count; one that does not fit is skipped for a smaller one.
    fn act(&mut self, signal: &Signal, skills: Vec<String>) -> Vec<Effect> {
        let budget = self.config.budget;
        let agent = self.agent(&signal.agent_id);
        let mut spent: u64 = 0;
        let mut chosen = Vec::new();

        for skill in skills {
            let Some(bytes) = agent
                .offerable()
                .iter()
                .find(|offered| offered.id == skill)
                .map(|offered| u64::from(offered.bytes))
            else {
                continue;
            };

            if chosen.len() >= budget.skills || spent.saturating_add(bytes) > budget.bytes {
                continue;
            }

            spent = spent.saturating_add(bytes);
            chosen.push(skill);
        }

        agent.attached.extend(chosen.iter().cloned());

        chosen.iter().map(|skill| context(signal, skill)).collect()
    }
}

/// A prompt's skills join the user's message; a skill the agent asked for
/// answers it in the running turn; a drift hand-over reaches the running
/// turn, or waits for the next one once the turn has ended.
fn context(signal: &Signal, skill: &str) -> Effect {
    let (delivery, text) = match signal.kind {
        SignalKind::UserMessage { .. } => (Delivery::Prompt, None),
        SignalKind::AgentRequest { .. } => (Delivery::Steer, None),
        SignalKind::TurnEnd { .. } => (Delivery::Wait, Some(drift_text(skill))),
        _ => (Delivery::Steer, Some(drift_text(skill))),
    };

    Effect::Context {
        agent_id: signal.agent_id.clone(),
        delivery,
        label: format!("skill {skill}"),
        skills: vec![skill.to_string()],
        text,
    }
}

fn drift_text(skill: &str) -> String {
    format!("Chauffeur: the {skill} skill fits this work better than the current approach.")
}

const DRIFT_INSTRUCTIONS: &str = "Which offered skill, if any, directly improves the exact \
    action in the agent's recent tool result? Choose none if the approach already works. \
    A skill covering the same topic is not evidence of misuse; successful gh use for GitHub \
    does not call for a browser or browser-harness hand-over.";

fn choice(id: &str, instructions: &str, skills: &[&CatalogEntry]) -> Question {
    let mut options: Vec<ChoiceOption> = skills
        .iter()
        .map(|skill| ChoiceOption {
            value: skill.id.clone(),
            description: skill.description.clone(),
        })
        .collect();

    options.push(ChoiceOption {
        value: NONE.into(),
        description: "No skill clearly helps.".into(),
    });

    Question {
        id: id.into(),
        instructions: instructions.into(),
        kind: QuestionKind::Choice { options },
    }
}
