//! Skill exposure: System One picks the one skill, or none, that best helps.
//! It asks on each user message, and again after tool results and at turn end
//! in case the agent lost track, such as driving a browser where a dedicated
//! skill exists.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use chauffeur_core::{
    Answer, AnswerValue, Capability, CatalogEntry, ChoiceOption, Delivery, Effect, Plan, Question,
    QuestionKind, Signal, SignalKind, Situation,
};

pub const ID: &str = "skill-exposure";
/// The option that attaches nothing.
pub const NONE: &str = "none";
/// Attach only when the choice is at least this confident.
pub const MIN_CONFIDENCE: f32 = 0.4;
/// Mid-turn hand-overs need stronger evidence than prompt admission.
pub const DRIFT_MIN_CONFIDENCE: f32 = 0.7;
/// Largest skill that is attached, about 4k tokens.
pub const MAX_ATTACH_BYTES: u64 = 16_384;
/// Skills offered per question; the catalog is truncated in host order.
pub const MAX_OPTIONS: usize = 64;
/// Tool results between drift checks are skipped for this long, per agent.
pub const DRIFT_COOLDOWN_SECS: u64 = 60;
const MAX_AGENTS: usize = 256;
const PICK: &str = "pick";
const DRIFT: &str = "drift";
const BROWSER_HARNESS: &str = "browser-harness";

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

    fn drift_due(&self, at: u64) -> bool {
        self.last_drift_at
            .is_none_or(|last| at.saturating_sub(last) >= DRIFT_COOLDOWN_SECS)
    }

    /// Remember the latest tool action and whether it drove a browser, so a
    /// browser hand-over is offered only for actual browser use.
    fn record_tool(&mut self, tool: &str, ok: bool, input: &str) {
        self.tools_seen = self.tools_seen.saturating_add(1);
        self.last_tool = Some(format!("tool: {tool}; succeeded: {ok}; input: {input}"));
        self.browser_tool = tool.starts_with("browser")
            || (matches!(tool, "shell" | "execute")
                && (input.contains(BROWSER_HARNESS) || input.contains("tools.browser.")));
    }

    /// Whether `skill` may be offered by the question `id`.
    fn admits(&self, id: &str, skill: &str) -> bool {
        id != DRIFT || skill != BROWSER_HARNESS || self.browser_tool
    }
}

#[derive(Default)]
pub struct SkillExposure {
    agents: HashMap<String, Agent>,
}

impl SkillExposure {
    fn agent(&mut self, agent_id: &str) -> &mut Agent {
        if !self.agents.contains_key(agent_id) && self.agents.len() >= MAX_AGENTS {
            self.agents.clear();
        }

        self.agents.entry(agent_id.to_string()).or_default()
    }
}

impl Capability for SkillExposure {
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

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        let (id, instructions) = match &signal.kind {
            SignalKind::UserMessage { skills, .. } => {
                // The host lists only skills not yet attached in this context.
                *self.agent(&signal.agent_id) = Agent {
                    skills: skills.clone(),
                    ..Agent::default()
                };

                (PICK, PICK_INSTRUCTIONS)
            }
            SignalKind::ToolResult { .. } | SignalKind::TurnEnd { .. } => {
                let Some(agent) = self.agents.get_mut(&signal.agent_id) else {
                    return Plan::Skip;
                };
                let turn_end = matches!(signal.kind, SignalKind::TurnEnd { .. });

                if let SignalKind::ToolResult {
                    tool, ok, input, ..
                } = &signal.kind
                {
                    agent.record_tool(tool, *ok, input);
                }

                if agent.tools_seen == agent.drift_seen
                    || (!turn_end && !agent.drift_due(signal.at))
                {
                    return Plan::Skip;
                }

                agent.last_drift_at = Some(signal.at);
                agent.drift_seen = agent.tools_seen;

                (DRIFT, DRIFT_INSTRUCTIONS)
            }
            _ => return Plan::Skip,
        };
        let Some(agent) = self.agents.get(&signal.agent_id) else {
            return Plan::Skip;
        };
        let offerable: Vec<_> = agent
            .offerable()
            .into_iter()
            .filter(|skill| agent.admits(id, &skill.id))
            .collect();

        if offerable.is_empty() {
            return Plan::Skip;
        }

        let instructions = if id == DRIFT {
            format!(
                "{instructions} Latest tool action: {}",
                agent.last_tool.as_deref().unwrap_or("unknown")
            )
        } else {
            instructions.to_string()
        };

        Plan::Ask(vec![question(id, &instructions, &offerable)])
    }

    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        // Enhancements fail open: a failed judgment attaches nothing.
        let Some(answer) = answers.and_then(|answers| {
            answers
                .iter()
                .find(|answer| answer.id == PICK || answer.id == DRIFT)
        }) else {
            return Vec::new();
        };
        let AnswerValue::Choice(choice) = &answer.value else {
            return Vec::new();
        };

        let minimum = if answer.id == DRIFT {
            DRIFT_MIN_CONFIDENCE
        } else {
            MIN_CONFIDENCE
        };

        if choice == NONE || answer.effective_confidence() < minimum {
            return Vec::new();
        }

        let agent = self.agent(&signal.agent_id);

        if !agent.admits(&answer.id, choice) {
            return Vec::new();
        }

        let fits = agent
            .offerable()
            .iter()
            .any(|skill| skill.id == *choice && u64::from(skill.bytes) <= MAX_ATTACH_BYTES);

        if !fits {
            return Vec::new();
        }

        agent.attached.insert(choice.clone());

        vec![context(signal, choice)]
    }
}

/// A picked skill joins the user's message; a drift hand-over reaches the
/// running turn, or waits for the next one once the turn has ended.
fn context(signal: &Signal, skill: &str) -> Effect {
    let (delivery, text) = match signal.kind {
        SignalKind::UserMessage { .. } => (Delivery::Prompt, None),
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

const PICK_INSTRUCTIONS: &str = "Which one skill would most help the coding agent act on the \
    user's latest request? Choose none unless a skill clearly helps.";

const DRIFT_INSTRUCTIONS: &str = "Which offered skill, if any, directly improves the exact \
    action in the agent's recent tool result? Choose none if the approach already works. \
    A skill covering the same topic is not evidence of misuse; successful gh use for GitHub \
    does not call for a browser or browser-harness hand-over.";

fn question(id: &str, instructions: &str, skills: &[&CatalogEntry]) -> Question {
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
