//! Permission capability: every loaded skill contract that matches a
//! permission request checks its deterministic evidence, asks its one typed
//! question, and the most restrictive outcome answers the host.

pub mod contract;
mod evidence;

use std::collections::HashMap;

use chauffeur_core::{
    Answer, Capability, ChoiceOption, Effect, PermissionDecision, Plan, Question, QuestionKind,
    Signal, SignalKind, Situation,
};

use contract::{Branch, DecisionQuestion, Effect as OutcomeEffect, Outcome, QuestionType};
pub use contract::{Skill, load_skill, load_skills};

pub const ID: &str = "permission";
/// The host event every permission request arrives as.
pub const EVENT: &str = "permission.evaluate";
const MAX_COOLDOWNS: usize = 4_096;

pub struct Permission {
    contracts: Vec<Skill>,
    /// Last evaluation time per (agent, contract).
    cooldowns: HashMap<(String, String), u64>,
}

/// How a contract was resolved for one request.
enum Judgment<'a> {
    /// No question was asked (settled by evidence or cooldown).
    NotAsked,
    Failed,
    Answered(&'a [Answer]),
}

impl Permission {
    #[must_use]
    pub fn new(contracts: Vec<Skill>) -> Self {
        Self {
            contracts,
            cooldowns: HashMap::new(),
        }
    }

    fn matching(&self, action: &str) -> Vec<&Skill> {
        self.contracts
            .iter()
            .filter(|skill| {
                skill
                    .match_conditions
                    .events
                    .iter()
                    .any(|event| event == EVENT)
                    && skill
                        .match_conditions
                        .actions
                        .iter()
                        .any(|allowed| allowed == action)
            })
            .collect()
    }

    fn cooling(&self, agent_id: &str, skill: &Skill, at: u64) -> bool {
        self.cooldowns
            .get(&(agent_id.to_string(), skill.identity.id.clone()))
            .is_some_and(|last| at.saturating_sub(*last) < skill.cooldown_seconds)
    }

    /// Each matching contract's outcome for this request, plus the contracts
    /// that must be recorded against their cooldown.
    fn outcomes<'a>(
        &'a self,
        signal: &Signal,
        action: &str,
        judgment: &Judgment<'_>,
    ) -> (Vec<&'a Outcome>, Vec<String>) {
        let evidence = evidence::collect(&signal.kind);
        let mut outcomes = Vec::new();
        let mut record = Vec::new();

        for skill in self.matching(action) {
            let id = skill.identity.id.clone();

            if !contract::missing_evidence(skill, &evidence).is_empty() {
                outcomes.push(&skill.outcomes.missing_evidence);
                record.push(id);
            } else if self.cooling(&signal.agent_id, skill, signal.at) {
                outcomes.push(&skill.outcomes.cooldown);
            } else {
                outcomes.push(judged(skill, judgment));
                record.push(id);
            }
        }

        (outcomes, record)
    }

    fn record(&mut self, agent_id: &str, contracts: Vec<String>, at: u64) {
        for id in contracts {
            let key = (agent_id.to_string(), id);

            if !self.cooldowns.contains_key(&key) && self.cooldowns.len() >= MAX_COOLDOWNS {
                self.cooldowns.clear();
            }

            self.cooldowns.insert(key, at);
        }
    }

    fn answer(&mut self, signal: &Signal, action: &str, judgment: &Judgment<'_>) -> Vec<Effect> {
        let (outcomes, record) = self.outcomes(signal, action, judgment);
        let effect = combine(&signal.agent_id, &outcomes);

        self.record(&signal.agent_id, record, signal.at);

        effect.into_iter().collect()
    }
}

impl Capability for Permission {
    fn save(&self) -> Option<serde_json::Value> {
        serde_json::to_value(self.cooldowns.iter().collect::<Vec<_>>()).ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        if let Ok(cooldowns) = serde_json::from_value::<Vec<((String, String), u64)>>(state) {
            self.cooldowns = cooldowns.into_iter().take(MAX_COOLDOWNS).collect();
        }
    }

    fn id(&self) -> &str {
        ID
    }

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        let SignalKind::PermissionRequest { action, .. } = &signal.kind else {
            return Plan::Skip;
        };
        let evidence = evidence::collect(&signal.kind);
        let questions: Vec<Question> = self
            .matching(action)
            .into_iter()
            .filter(|skill| contract::missing_evidence(skill, &evidence).is_empty())
            .filter(|skill| !self.cooling(&signal.agent_id, skill, signal.at))
            .map(question)
            .collect();

        if questions.is_empty() {
            let action = action.clone();

            return match self.answer(signal, &action, &Judgment::NotAsked) {
                effects if effects.is_empty() => Plan::Skip,
                effects => Plan::Settled(effects),
            };
        }

        Plan::Ask(questions)
    }

    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        let SignalKind::PermissionRequest { action, .. } = &signal.kind else {
            return Vec::new();
        };
        let judgment = answers.map_or(Judgment::Failed, Judgment::Answered);

        self.answer(signal, &action.clone(), &judgment)
    }
}

fn judged<'a>(skill: &'a Skill, judgment: &Judgment<'_>) -> &'a Outcome {
    let branch = match judgment {
        Judgment::Answered(answers) => answers
            .iter()
            .find(|answer| answer.id == skill.identity.id)
            .and_then(|answer| contract::resolve_branch(skill, answer).ok()),
        Judgment::NotAsked | Judgment::Failed => None,
    };

    match branch {
        Some(Branch::Positive) => &skill.outcomes.positive,
        Some(Branch::Negative) => &skill.outcomes.negative,
        Some(Branch::Uncertain) => &skill.outcomes.uncertain,
        None => &skill.outcomes.judge_failure,
    }
}

/// The most restrictive outcome wins: deny, then ask, then allow. Prompt and
/// remind outcomes ask, carrying their reminder.
fn combine(agent_id: &str, outcomes: &[&Outcome]) -> Option<Effect> {
    let chosen = outcomes.iter().min_by_key(|outcome| rank(outcome.effect))?;
    let decision = match chosen.effect {
        OutcomeEffect::Allow => PermissionDecision::Allow,
        OutcomeEffect::Deny => PermissionDecision::Deny,
        OutcomeEffect::Ask | OutcomeEffect::Prompt | OutcomeEffect::Remind => {
            PermissionDecision::Ask
        }
    };
    let message = (decision != PermissionDecision::Allow).then(|| {
        chosen
            .reminder
            .clone()
            .unwrap_or_else(|| chosen.message.clone())
    });

    Some(Effect::Permission {
        agent_id: agent_id.to_string(),
        decision,
        message,
    })
}

fn rank(effect: OutcomeEffect) -> u8 {
    match effect {
        OutcomeEffect::Deny => 0,
        OutcomeEffect::Ask | OutcomeEffect::Prompt | OutcomeEffect::Remind => 1,
        OutcomeEffect::Allow => 2,
    }
}

fn question(skill: &Skill) -> Question {
    let decision: &DecisionQuestion = &skill.decision;
    let kind = match decision.kind {
        QuestionType::Choice => QuestionKind::Choice {
            options: decision
                .criteria
                .iter()
                .map(|criterion| ChoiceOption {
                    value: criterion.value.clone(),
                    description: criterion.description.clone(),
                })
                .collect(),
        },
        QuestionType::Score => QuestionKind::Score {
            levels: decision
                .criteria
                .iter()
                .map(|criterion| criterion.value.clone())
                .collect(),
        },
        QuestionType::Noul => QuestionKind::Noul,
    };

    Question {
        id: skill.identity.id.clone(),
        instructions: decision.instructions.clone(),
        kind,
    }
}
