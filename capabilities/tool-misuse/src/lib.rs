//! Tool misuse: after a tool call, every misuse contract watching that tool
//! asks System One whether the call is that misuse; a confirmed one steers the
//! agent, optionally handing over a better-fitting skill. Tools no contract
//! watches are never judged.

mod contract;

use std::collections::HashMap;

pub use contract::{Identity, Match, MisuseContract, load_contracts};

use chauffeur_core::{
    Answer, AnswerValue, Capability, Delivery, Effect, Plan, Question, QuestionKind, Signal,
    SignalKind, Situation,
};

pub const ID: &str = "tool-misuse";
const MAX_TRACKED: usize = 4_096;

pub struct ToolMisuse {
    contracts: Vec<MisuseContract>,
    /// Last nudge per (agent, contract).
    last_nudge: HashMap<(String, String), u64>,
}

impl ToolMisuse {
    #[must_use]
    pub fn new(contracts: Vec<MisuseContract>) -> Self {
        Self {
            contracts,
            last_nudge: HashMap::new(),
        }
    }

    /// Contracts watching `tool` that are not cooling down for this agent.
    fn watching<'a>(
        &'a self,
        signal: &'a Signal,
        tool: &'a str,
    ) -> impl Iterator<Item = &'a MisuseContract> {
        self.contracts.iter().filter(move |contract| {
            contract.watches(tool)
                && self
                    .last_nudge
                    .get(&(signal.agent_id.clone(), contract.identity.id.clone()))
                    .is_none_or(|last| signal.at.saturating_sub(*last) >= contract.cooldown_seconds)
        })
    }
}

impl Capability for ToolMisuse {
    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.last_nudge.iter().collect::<Vec<_>>()).ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        if let Ok(nudges) = serde_json::from_value::<Vec<((String, String), u64)>>(state) {
            self.last_nudge = nudges.into_iter().take(MAX_TRACKED).collect();
        }
    }

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        let SignalKind::ToolResult {
            tool, ok, input, ..
        } = &signal.kind
        else {
            return Plan::Skip;
        };
        let outcome = if *ok { "succeeded" } else { "failed" };
        let questions: Vec<Question> = self
            .watching(signal, tool)
            .map(|contract| Question {
                id: contract.identity.id.clone(),
                // The call comes first so the contract can ask about "this call".
                instructions: format!(
                    "The agent's latest {tool} call {outcome} with input: {input}\n{}\nA call the \
                     user explicitly asked for, exactly as asked, does not count.",
                    contract.question
                ),
                kind: QuestionKind::Noul,
            })
            .collect();

        if questions.is_empty() {
            return Plan::Skip;
        }

        Plan::Ask(questions)
    }

    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        // A failed judgment stays silent.
        let (SignalKind::ToolResult { tool, .. }, Some(answers)) = (&signal.kind, answers) else {
            return Vec::new();
        };
        let confirmed: Vec<MisuseContract> = self
            .watching(signal, tool)
            .filter(|contract| {
                answers.iter().any(|answer| {
                    answer.id == contract.identity.id
                        && matches!(answer.value, AnswerValue::Noul(p) if p >= contract.nudge_at_or_above)
                        && answer.effective_confidence() >= contract.minimum_confidence
                })
            })
            .cloned()
            .collect();
        let mut effects = Vec::new();

        for contract in confirmed {
            if self.last_nudge.len() >= MAX_TRACKED {
                self.last_nudge.clear();
            }

            self.last_nudge.insert(
                (signal.agent_id.clone(), contract.identity.id.clone()),
                signal.at,
            );
            // The steer, with any hand-over skill, reaches the running turn.
            effects.push(Effect::Context {
                agent_id: signal.agent_id.clone(),
                delivery: Delivery::Steer,
                label: format!("{tool} check"),
                skills: contract.handoff_skill.into_iter().collect(),
                text: Some(format!("Chauffeur: {}", contract.steer)),
            });
        }

        effects
    }
}
