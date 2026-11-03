//! Skill exposure: on each user message, and when the agent asks for help
//! itself, System One judges every offered skill in one call and each one the
//! request needs is attached. After tool results and at turn end it picks at
//! most one hand-over, in case the agent lost track, such as driving a
//! browser where a dedicated skill exists. Run it as
//! `Judging::new(SkillExposure::new(config))`, or with the shipped config as
//! `Judging::new(SkillExposure::default())`.

mod config;
mod fanout;

#[cfg(feature = "sources")]
pub mod sources;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use chauffeur_core::judgment::strategy::{self, Ranked};
use chauffeur_core::{
    CatalogEntry, ChoiceOption, Delivery, Effect, Facts, Judged, Judgment, Question, QuestionKind,
    SecondOpinion, Signal, SignalKind, Situation,
};

pub use config::{Budget, Drift, Ranking, SkillExposureConfig, SkillTexts};

pub const ID: &str = "skill-exposure";
/// The drift option that hands over nothing.
pub const NONE: &str = chauffeur_core::judgment::NONE;
/// Skills judged per signal. Beyond it, pinned and better-ranked skills win.
pub const MAX_OPTIONS: usize = 64;
const MAX_AGENTS: usize = 256;
const DRIFT: &str = "drift";
/// The question that picks among several skills serving the agent's request.
const BEST: &str = "best";
/// Skills that question chooses among, likeliest first.
const FINALISTS: usize = 4;
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
    /// The latest prompt's skills, most similar first, when the daemon ranked them.
    #[serde(default)]
    ranking: Vec<String>,
}

impl Agent {
    /// Every skill still to offer, ranked ones first in rank order.
    fn offerable(&self) -> Vec<&CatalogEntry> {
        let mut seen = HashSet::new();
        let mut offerable: Vec<&CatalogEntry> = self
            .skills
            .iter()
            .filter(|skill| {
                skill.id != NONE
                    && !self.attached.contains(&skill.id)
                    && seen.insert(skill.id.as_str())
            })
            .collect();

        offerable.sort_by_key(|skill| fanout::rank(&self.ranking, &skill.id));
        offerable
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

/// The skills to attach, and for an agent's request that none serves, the
/// closest ones, to name without attaching.
#[derive(Debug, Default, PartialEq)]
pub struct Picked {
    pub skills: Vec<String>,
    pub closest: Vec<String>,
}

impl Picked {
    fn skills(skills: Vec<String>) -> Self {
        Self {
            skills,
            closest: Vec::new(),
        }
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

    /// The pinned and closest-ranked skills, judged in one round.
    fn fan_out(&self, signal: &Signal) -> Option<Judgment<Vec<String>>> {
        let agent = self.agents.get(&signal.agent_id)?;
        let judged = fanout::judged(signal, &agent.offerable(), &self.config);
        let candidates = fanout::candidates(signal, &judged, &self.config);

        (!candidates.is_empty()).then(|| strategy::fan_out(candidates))
    }

    /// The agent's request: the skills that serve it, and when more do than
    /// it may be handed, a second question picks the one made for exactly
    /// the need; when none does, the closest few by name.
    fn request(&self, signal: &Signal) -> Option<Judgment<Picked>> {
        let SignalKind::AgentRequest { need, .. } = &signal.kind else {
            return None;
        };
        let agent = self.agents.get(&signal.agent_id)?;
        let judged = fanout::judged(signal, &agent.offerable(), &self.config);
        let candidates = fanout::candidates(signal, &judged, &self.config);

        if candidates.is_empty() {
            return None;
        }

        let limits = self.config.request;
        let texts = self.config.texts.clone();
        let instructions = texts.request_choice.render(&[("need", need)]);
        let offered: Vec<CatalogEntry> = judged.into_iter().cloned().collect();

        Some(
            strategy::ranked(candidates).then(move |ranked: Vec<Ranked<String>>| {
                let held: Vec<String> = ranked
                    .iter()
                    .filter(|candidate| candidate.held)
                    .map(|candidate| candidate.key.clone())
                    .collect();

                if held.is_empty() {
                    let closest = ranked
                        .into_iter()
                        .filter(|candidate| candidate.p >= limits.closest_at)
                        .take(limits.closest)
                        .map(|candidate| candidate.key)
                        .collect();

                    return Judgment::done(Picked {
                        skills: Vec::new(),
                        closest,
                    });
                }

                if held.len() <= limits.skills {
                    return Judgment::done(Picked::skills(held));
                }

                let finalists: Vec<&CatalogEntry> = held
                    .iter()
                    .take(FINALISTS)
                    .filter_map(|id| offered.iter().find(|skill| skill.id == *id))
                    .collect();
                let question = Question {
                    second_opinion: SecondOpinion::Never,
                    ..choice(BEST, &instructions, &finalists, &texts)
                };
                let pick = limits.confidence.pick();

                Judgment::ask(question, move |answer| {
                    Picked::skills(best_first(held, pick.chosen(answer), limits.skills))
                })
            }),
        )
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
    fn drift(&self, signal: &Signal) -> Option<Judgment<Vec<String>>> {
        let agent = self.agents.get(&signal.agent_id)?;
        let drift = &self.config.drift;
        let offerable: Vec<_> = agent
            .offerable()
            .into_iter()
            .filter(|skill| agent.browser_tool || !drift.browser_only.contains(&skill.id))
            .take(MAX_OPTIONS)
            .collect();
        let pick = drift.confidence.pick();

        if offerable.is_empty() {
            return None;
        }

        let texts = &self.config.texts;
        let action = agent
            .last_tool
            .clone()
            .unwrap_or_else(|| texts.unknown_action.as_str().to_string());
        let instructions = texts.drift.render(&[("action", &action)]);

        Some(
            Judgment::ask(
                choice(DRIFT, &instructions, &offerable, texts),
                move |answer| pick.chosen(answer),
            )
            .map(|skill| skill.into_iter().collect()),
        )
    }
}

impl Judged for SkillExposure {
    type Verdict = Picked;

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

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judgment<Picked>> {
        match &signal.kind {
            SignalKind::UserMessage {
                skills, ranking, ..
            } => {
                // The host lists only skills not yet attached in this context.
                *self.agent(&signal.agent_id) = Agent {
                    skills: skills.clone(),
                    ranking: ranking.clone(),
                    ..Agent::default()
                };

                self.fan_out(signal)
                    .map(|judgment| judgment.map(Picked::skills))
            }
            SignalKind::AgentRequest { .. } => self.request(signal),
            SignalKind::ToolResult { .. } | SignalKind::TurnEnd { .. }
                if self.drift_due(signal) =>
            {
                self.drift(signal)
                    .map(|judgment| judgment.map(Picked::skills))
            }
            _ => None,
        }
    }

    /// Attach the admitted skills in order while they fit the signal's byte
    /// budget and count; one that does not fit is skipped for a smaller one.
    /// The closest skills are named, not attached.
    fn act(&mut self, signal: &Signal, picked: Picked) -> Vec<Effect> {
        let Picked { skills, closest } = picked;
        let budget = self.config.budget;
        let texts = &self.config.texts;
        let suggestion = (!closest.is_empty()).then(|| Effect::Suggest {
            agent_id: signal.agent_id.clone(),
            names: closest
                .iter()
                .map(|skill| texts.closest.render(&[("skill", skill)]))
                .collect(),
        });
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

        chosen
            .iter()
            .map(|skill| context(signal, skill, &self.config.texts))
            .chain(suggestion)
            .collect()
    }
}

/// The `limit` skills to hand over: the chosen one first when it is one of
/// `held`, then the likeliest others.
fn best_first(mut held: Vec<String>, chosen: Option<String>, limit: usize) -> Vec<String> {
    if let Some(at) = chosen.and_then(|chosen| held.iter().position(|skill| *skill == chosen)) {
        let best = held.remove(at);

        held.insert(0, best);
    }

    held.truncate(limit);
    held
}

/// A prompt's skills join the user's message; a skill the agent asked for
/// answers it in the running turn; a drift hand-over reaches the running
/// turn, or waits for the next one once the turn has ended.
fn context(signal: &Signal, skill: &str, texts: &SkillTexts) -> Effect {
    let notice = || texts.drift_notice.render(&[("skill", skill)]);
    let (delivery, text) = match signal.kind {
        SignalKind::UserMessage { .. } => (Delivery::Prompt, None),
        SignalKind::AgentRequest { .. } => (Delivery::Steer, None),
        SignalKind::TurnEnd { .. } => (Delivery::Wait, Some(notice())),
        _ => (Delivery::Steer, Some(notice())),
    };

    Effect::Context {
        agent_id: signal.agent_id.clone(),
        delivery,
        label: texts.label.render(&[("skill", skill)]),
        skills: vec![skill.to_string()],
        text,
    }
}

fn choice(id: &str, instructions: &str, skills: &[&CatalogEntry], texts: &SkillTexts) -> Question {
    let mut options: Vec<ChoiceOption> = skills
        .iter()
        .map(|skill| ChoiceOption {
            value: skill.id.clone(),
            description: skill.description.clone(),
        })
        .collect();

    options.push(ChoiceOption {
        value: NONE.into(),
        description: texts.no_skill.as_str().to_string(),
    });

    Question {
        id: id.into(),
        instructions: instructions.into(),
        kind: QuestionKind::Choice { options },
        facts: Facts::new(),
        second_opinion: Default::default(),
    }
}
