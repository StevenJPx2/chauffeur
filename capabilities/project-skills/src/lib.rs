//! Project-local JSON skills: bounded facts admit contracts, Jev judges their
//! ordered steps, and confirmed contracts deliver context without loading an
//! agent skill body.

mod contract;

use std::collections::{HashMap, HashSet};

use chauffeur_core::{
    Answer, AnswerValue, Capability, Effect, PipeStep, Plan, Question, QuestionKind, Signal,
    SignalKind, Situation,
};
use serde::{Deserialize, Serialize};

pub use contract::{Skill, load};

pub const ID: &str = "project-skills";
const MAX_AGENTS: usize = 256;
const MAX_TOOLS: usize = 256;
const MAX_FIRED: usize = 4_096;
const MAX_EFFECTS: usize = 2;

/// A once-skill that fired, keyed by (agent, workspace, skill).
type Fired = (String, String, String);

/// The skills admitted at the latest turn end, those whose first step was
/// confirmed and await their second, and effects already settled in round 1.
struct Active {
    skills: Vec<Skill>,
    next: Vec<usize>,
    /// `PipeStep::Next` carries no effects, so round 1 holds them for round 2.
    held: Vec<Effect>,
}

/// Loads the workspace's project skills at each turn end and delivers the
/// follow-through of every skill whose steps Jev confirms.
#[derive(Default)]
pub struct ProjectSkills {
    tools: HashMap<String, Tracked>,
    fired: HashSet<Fired>,
    active: Option<Active>,
}

#[derive(Deserialize, Serialize)]
struct Saved {
    tools: HashMap<String, Tracked>,
    fired: Vec<Fired>,
}

/// Tools one agent called in its current workspace since its last user
/// message.
#[derive(Clone, Deserialize, Serialize)]
struct Tracked {
    workspace: String,
    tools: HashSet<String>,
}

impl ProjectSkills {
    fn record_tool(&mut self, agent: &str, workspace: &str, tool: &str) {
        if workspace.is_empty() {
            return;
        }

        if !self.tools.contains_key(agent) && self.tools.len() >= MAX_AGENTS {
            self.tools.clear();
        }

        let tracked = self
            .tools
            .entry(agent.to_string())
            .or_insert_with(|| Tracked {
                workspace: workspace.into(),
                tools: HashSet::new(),
            });

        if tracked.workspace != workspace {
            tracked.workspace = workspace.into();
            tracked.tools.clear();
        }
        if tracked.tools.len() < MAX_TOOLS {
            tracked.tools.insert(tool.to_string());
        }
    }

    fn admitted(&self, agent: &str, workspace: &str, skill: &Skill) -> bool {
        let tools = self
            .tools
            .get(agent)
            .filter(|tracked| tracked.workspace == workspace)
            .map(|tracked| &tracked.tools);
        let called = |name: &String| tools.is_some_and(|tools| tools.contains(name));
        let matching = &skill.matching;
        let any =
            matching.tools_called_any.is_empty() || matching.tools_called_any.iter().any(called);
        let none = !matching.tools_not_called.iter().any(called);
        let fired = skill.once
            && self
                .fired
                .contains(&(agent.to_string(), workspace.to_string(), skill.id.clone()));

        matches!(matching.event, contract::Event::TurnEnd) && any && none && !fired
    }

    fn deliver(&mut self, agent: &str, workspace: &str, skill: &Skill) -> Effect {
        if skill.once {
            if self.fired.len() >= MAX_FIRED {
                self.fired.clear();
            }

            self.fired
                .insert((agent.to_string(), workspace.to_string(), skill.id.clone()));
        }

        Effect::Context {
            agent_id: agent.to_string(),
            delivery: skill.effect.delivery,
            label: skill.effect.label.clone(),
            skills: Vec::new(),
            text: Some(skill.effect.text.clone()),
        }
    }

    fn turn_end(&mut self, agent: &str, workspace: &str, user_request: &str) -> Plan {
        let skills: Vec<Skill> = match contract::load(workspace) {
            Ok(skills) => skills
                .into_iter()
                .filter(|skill| self.admitted(agent, workspace, skill))
                .collect(),
            Err(error) => {
                eprintln!("chauffeur: project skills unavailable: {error}");
                return Plan::Skip;
            }
        };
        let questions: Vec<Question> = skills
            .iter()
            .filter_map(|skill| question(skill, 0, user_request))
            .collect();

        if questions.is_empty() {
            return Plan::Skip;
        }

        self.active = Some(Active {
            skills,
            next: Vec::new(),
            held: Vec::new(),
        });

        Plan::Ask(questions)
    }
}

impl Capability for ProjectSkills {
    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        serde_json::to_value(Saved {
            tools: self.tools.clone(),
            fired: self.fired.iter().cloned().collect(),
        })
        .ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        let Ok(saved) = serde_json::from_value::<Saved>(state) else {
            return;
        };

        self.tools = saved
            .tools
            .into_iter()
            .take(MAX_AGENTS)
            .map(|(agent, tracked)| {
                let tools = tracked.tools.into_iter().take(MAX_TOOLS).collect();

                (agent, Tracked { tools, ..tracked })
            })
            .collect();
        self.fired = saved.fired.into_iter().take(MAX_FIRED).collect();
    }

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        self.active = None;

        match &signal.kind {
            SignalKind::UserMessage { .. } => {
                self.tools.remove(&signal.agent_id);
                self.fired.retain(|(agent, _, _)| agent != &signal.agent_id);
                Plan::Skip
            }
            SignalKind::ToolResult {
                tool, workspace, ..
            } => {
                self.record_tool(&signal.agent_id, workspace, tool);
                Plan::Skip
            }
            SignalKind::TurnEnd {
                workspace,
                user_request,
            } => self.turn_end(&signal.agent_id, workspace, user_request),
            _ => Plan::Skip,
        }
    }

    fn decide(&mut self, _: &Signal, _: Option<&[Answer]>) -> Vec<Effect> {
        Vec::new()
    }

    /// Round 1 judges every admitted skill's first step; round 2 judges the
    /// second step of those that passed. A skill delivers once its last step
    /// is confirmed.
    fn advance(&mut self, signal: &Signal, answers: Option<&[Answer]>, round: usize) -> PipeStep {
        let Some(mut active) = self.active.take() else {
            return PipeStep::Done(Vec::new());
        };
        let SignalKind::TurnEnd {
            workspace,
            user_request,
        } = &signal.kind
        else {
            return PipeStep::Done(Vec::new());
        };
        let step = round.saturating_sub(1);
        let indices: Vec<usize> = if step == 0 {
            (0..active.skills.len()).collect()
        } else {
            std::mem::take(&mut active.next)
        };
        let mut effects = std::mem::take(&mut active.held);
        let mut next = Vec::new();

        for index in indices {
            let Some(skill) = active.skills.get(index) else {
                continue;
            };

            if !confirmed(skill, step, answers) {
                continue;
            }
            if step.saturating_add(1) < skill.steps.len() {
                next.push(index);
            } else if effects.len() < MAX_EFFECTS {
                effects.push(self.deliver(&signal.agent_id, workspace, skill));
            }
        }

        let questions: Vec<Question> = next
            .iter()
            .filter_map(|index| active.skills.get(*index))
            .filter_map(|skill| question(skill, step.saturating_add(1), user_request))
            .collect();

        if questions.is_empty() {
            return PipeStep::Done(effects);
        }

        active.next = next;
        active.held = effects;
        self.active = Some(active);

        PipeStep::Next(questions)
    }
}

/// Whether Jev confirmed `skill`'s step at `index`.
fn confirmed(skill: &Skill, index: usize, answers: Option<&[Answer]>) -> bool {
    let (Some(step), Some(answers)) = (skill.steps.get(index), answers) else {
        return false;
    };
    let id = format!("{}/{}", skill.id, step.id);

    answers.iter().any(|answer| {
        answer.id == id
            && matches!(answer.value, AnswerValue::Noul(p) if p >= step.yes_at_or_above)
            && answer.effective_confidence() >= step.minimum_confidence
    })
}

fn question(skill: &Skill, index: usize, user_request: &str) -> Option<Question> {
    let step = skill.steps.get(index)?;

    Some(Question {
        id: format!("{}/{}", skill.id, step.id),
        instructions: format!(
            "{} Latest user request: {user_request}. Respect explicit user instructions.",
            step.question
        ),
        kind: QuestionKind::Noul,
    })
}
