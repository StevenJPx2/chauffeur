//! Idle reminder: when an agent's turn ends, every plugin rule whose gate
//! admits the agent's structural facts (the tools it ran and the integration
//! events it received) asks System One whether its situation holds; the
//! highest-priority confirmed rules become reminders.

mod plugin;
mod rule;

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

pub use plugin::{Plugin, compose};
pub use rule::{Gate, IdleFacts, Rule, Threshold, load_rules};

use chauffeur_core::{
    Answer, AnswerValue, Capability, Delivery, Effect, Plan, Question, QuestionKind, Signal,
    SignalKind, Situation,
};

pub const ID: &str = "idle-reminder";
/// Reminders delivered per idle turn, at most.
pub const MAX_REMINDERS: usize = 2;
const MAX_AGENTS: usize = 256;
const MAX_TOOLS_PER_AGENT: usize = 256;
const MAX_HOOKS_PER_AGENT: usize = 64;

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
struct Fired {
    count: u32,
    last_at: u64,
}

pub struct IdleReminder {
    rules: Vec<Rule>,
    /// Distinct tools each agent has run.
    tools: HashMap<String, HashSet<String>>,
    /// Distinct integration events each agent has seen, as `source:kind`.
    hooks: HashMap<String, Vec<String>>,
    fired: HashMap<(String, String), Fired>,
}

impl IdleReminder {
    #[must_use]
    pub fn new(rules: Vec<Rule>) -> Self {
        Self {
            rules,
            tools: HashMap::new(),
            hooks: HashMap::new(),
            fired: HashMap::new(),
        }
    }

    fn record_tool(&mut self, agent_id: &str, tool: &str) {
        if !self.tools.contains_key(agent_id) && self.tools.len() >= MAX_AGENTS {
            self.tools.clear();
        }

        let tools = self.tools.entry(agent_id.to_string()).or_default();

        if tools.len() < MAX_TOOLS_PER_AGENT {
            tools.insert(tool.to_string());
        }
    }

    fn record_hook(&mut self, agent_id: &str, hook: String) {
        if !self.hooks.contains_key(agent_id) && self.hooks.len() >= MAX_AGENTS {
            self.hooks.clear();
        }

        let hooks = self.hooks.entry(agent_id.to_string()).or_default();

        if hooks.len() < MAX_HOOKS_PER_AGENT && !hooks.contains(&hook) {
            hooks.push(hook);
        }
    }

    /// Rules whose gate admits the agent and whose once/cooldown allow firing.
    fn candidates(&self, signal: &Signal) -> Vec<&Rule> {
        let facts = IdleFacts::from_tools(
            self.tools
                .get(&signal.agent_id)
                .cloned()
                .unwrap_or_default(),
        )
        .with_hooks(
            self.hooks
                .get(&signal.agent_id)
                .cloned()
                .unwrap_or_default(),
        );

        self.rules
            .iter()
            .filter(|rule| rule.gate.admits(&facts))
            .filter(|rule| self.may_fire(&signal.agent_id, rule, signal.at))
            .collect()
    }

    fn may_fire(&self, agent_id: &str, rule: &Rule, at: u64) -> bool {
        let Some(previous) = self.fired.get(&(agent_id.to_string(), rule.id.clone())) else {
            return true;
        };

        !(rule.once && previous.count > 0)
            && at.saturating_sub(previous.last_at) >= rule.cooldown_secs
    }

    fn record_fired(&mut self, agent_id: &str, rule_id: &str, at: u64) {
        let key = (agent_id.to_string(), rule_id.to_string());

        if !self.fired.contains_key(&key) && self.fired.len() >= MAX_AGENTS.saturating_mul(16) {
            self.fired.clear();
        }

        let entry = self.fired.entry(key).or_default();

        entry.count = entry.count.saturating_add(1);
        entry.last_at = at;
    }
}

#[derive(Deserialize, Serialize)]
struct Saved {
    tools: HashMap<String, HashSet<String>>,
    hooks: HashMap<String, Vec<String>>,
    fired: Vec<((String, String), Fired)>,
}

impl Capability for IdleReminder {
    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        serde_json::to_value(Saved {
            tools: self.tools.clone(),
            hooks: self.hooks.clone(),
            fired: self
                .fired
                .iter()
                .map(|(key, fired)| (key.clone(), *fired))
                .collect(),
        })
        .ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        if let Ok(saved) = serde_json::from_value::<Saved>(state) {
            self.tools = saved.tools.into_iter().take(MAX_AGENTS).collect();
            self.hooks = saved.hooks.into_iter().take(MAX_AGENTS).collect();
            self.fired = saved
                .fired
                .into_iter()
                .take(MAX_AGENTS.saturating_mul(16))
                .collect();
        }
    }

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        match &signal.kind {
            SignalKind::ToolResult { tool, .. } => {
                self.record_tool(&signal.agent_id, tool);
                Plan::Skip
            }
            SignalKind::IntegrationEvent { source, kind, .. } => {
                self.record_hook(&signal.agent_id, format!("{source}:{kind}"));
                Plan::Skip
            }
            SignalKind::TurnEnd { .. } => {
                Plan::Ask(self.candidates(signal).into_iter().map(question).collect())
            }
            _ => Plan::Skip,
        }
    }

    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        // A failed judgment skips the nudge.
        let (SignalKind::TurnEnd { .. }, Some(answers)) = (&signal.kind, answers) else {
            return Vec::new();
        };
        let mut chosen: Vec<Rule> = self
            .candidates(signal)
            .into_iter()
            .filter(|rule| {
                answers.iter().any(|answer| {
                    answer.id == rule.id
                        && matches!(answer.value, AnswerValue::Noul(probability) if rule.threshold.accepts(probability))
                })
            })
            .cloned()
            .collect();

        chosen.sort_by_key(|rule| std::cmp::Reverse(rule.priority));
        chosen.truncate(MAX_REMINDERS);

        chosen
            .into_iter()
            .map(|rule| {
                self.record_fired(&signal.agent_id, &rule.id, signal.at);

                // A reminder wakes the idle agent.
                Effect::Context {
                    agent_id: signal.agent_id.clone(),
                    delivery: Delivery::Resume,
                    label: rule.id.clone(),
                    skills: Vec::new(),
                    text: Some(format!("📋 Reminder ({}):\n\n{}", rule.name, rule.reminder)),
                }
            })
            .collect()
    }
}

fn question(rule: &Rule) -> Question {
    Question {
        id: rule.id.clone(),
        instructions: rule.situation.clone(),
        kind: QuestionKind::Noul,
    }
}
