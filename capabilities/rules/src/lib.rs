//! Rules: on a tool result or a turn end, every rule whose gate admits the
//! agent's exact facts asks Jev its first step; a rule whose steps all hold
//! delivers its context, highest priority first. One format serves shipped
//! rules (`skills/rules/`) and a project's own (`.chauffeur/rules/`).

mod history;
mod rule;
mod source;

use std::collections::HashMap;

use chauffeur_core::{
    Answer, AnswerValue, Capability, Effect, PipeStep, Plan, Question, QuestionKind, Signal,
    SignalKind, Situation,
};
use serde::{Deserialize, Serialize};

use history::Histories;
pub use rule::{Gate, History, Rule, SCHEMA_VERSION, Step, Then, Trigger};
pub use source::{load_dir, load_project};

pub const ID: &str = "rules";
/// Contexts one signal delivers, at most.
pub const MAX_DELIVERIES: usize = 2;
const MAX_FIRED: usize = 4_096;

/// When a rule last delivered to an agent, and whether `once` is spent.
#[derive(Clone, Copy, Deserialize, Serialize)]
struct Fired {
    last_at: u64,
    spent: bool,
    /// A turn-window rule's `once` renews at the next user message.
    history: History,
}

/// The rules admitted for the signal being judged, those whose first step
/// held and await their second, and those confirmed so far.
struct Active {
    rules: Vec<Rule>,
    next: Vec<usize>,
    confirmed: Vec<usize>,
}

pub struct Rules {
    shipped: Vec<Rule>,
    histories: Histories,
    fired: HashMap<(String, String), Fired>,
    active: Option<Active>,
    /// The latest project-rule error, logged once until it changes.
    project_error: Option<String>,
}

#[derive(Deserialize)]
struct Saved {
    histories: Histories,
    fired: Vec<((String, String), Fired)>,
}

/// [`Saved`], borrowed for writing.
#[derive(Serialize)]
struct SavedRef<'a> {
    histories: &'a Histories,
    fired: Vec<(&'a (String, String), Fired)>,
}

impl Rules {
    /// `shipped` rules apply everywhere; project rules load per workspace.
    #[must_use]
    pub fn new(shipped: Vec<Rule>) -> Self {
        Self {
            shipped,
            histories: Histories::default(),
            fired: HashMap::new(),
            active: None,
            project_error: None,
        }
    }

    /// Shipped rules, then the workspace's own. A project rule that reuses a
    /// shipped ID is left out.
    fn rules_for(&mut self, workspace: &str) -> Vec<Rule> {
        let project = match load_project(workspace) {
            Ok(rules) => {
                self.project_error = None;
                rules
            }
            Err(error) => {
                if self.project_error.as_ref() != Some(&error) {
                    eprintln!("chauffeur: project rules unavailable: {error}");
                    self.project_error = Some(error);
                }
                Vec::new()
            }
        };
        let project: Vec<Rule> = project
            .into_iter()
            .filter(|rule| self.shipped.iter().all(|shipped| shipped.id != rule.id))
            .collect();

        self.shipped.iter().cloned().chain(project).collect()
    }

    fn may_fire(&self, agent: &str, rule: &Rule, at: u64) -> bool {
        self.fired
            .get(&(agent.to_string(), rule.id.clone()))
            .is_none_or(|fired| {
                !(rule.once && fired.spent)
                    && at.saturating_sub(fired.last_at) >= rule.cooldown_seconds
            })
    }

    /// Ask the first step of every rule `trigger` admits.
    fn start(
        &mut self,
        signal: &Signal,
        trigger: Trigger,
        workspace: &str,
        tool: Option<&str>,
    ) -> Plan {
        let agent = &signal.agent_id;
        let rules: Vec<Rule> = self
            .rules_for(workspace)
            .into_iter()
            .filter(|rule| rule.on == trigger)
            .filter(|rule| {
                tool.is_none_or(|tool| rule.when.tools.iter().any(|watched| watched == tool))
            })
            .filter(|rule| self.histories.admits(&rule.when, agent, workspace))
            .filter(|rule| self.may_fire(agent, rule, signal.at))
            .collect();
        let questions: Vec<Question> = rules
            .iter()
            .filter_map(|rule| question(rule, 0, &signal.kind))
            .collect();

        if questions.is_empty() {
            return Plan::Skip;
        }

        self.active = Some(Active {
            rules,
            next: Vec::new(),
            confirmed: Vec::new(),
        });

        Plan::Ask(questions)
    }

    /// The confirmed rules' contexts, highest priority first, within
    /// [`MAX_DELIVERIES`].
    fn deliver(&mut self, signal: &Signal, active: Active) -> Vec<Effect> {
        let mut confirmed: Vec<Rule> = active
            .confirmed
            .into_iter()
            .filter_map(|index| active.rules.get(index).cloned())
            .collect();

        confirmed.sort_by_key(|rule| std::cmp::Reverse(rule.priority));
        confirmed.truncate(MAX_DELIVERIES);

        confirmed
            .into_iter()
            .map(|rule| {
                self.record_fired(&signal.agent_id, &rule, signal.at);

                Effect::Context {
                    agent_id: signal.agent_id.clone(),
                    delivery: rule.then.delivery,
                    label: rule.label().to_string(),
                    skills: rule.then.skill.into_iter().collect(),
                    text: Some(rule.then.text),
                }
            })
            .collect()
    }

    fn record_fired(&mut self, agent: &str, rule: &Rule, at: u64) {
        let key = (agent.to_string(), rule.id.clone());

        if !self.fired.contains_key(&key) && self.fired.len() >= MAX_FIRED {
            self.fired.clear();
        }

        self.fired.insert(
            key,
            Fired {
                last_at: at,
                spent: true,
                history: rule.when.history,
            },
        );
    }

    /// A user message renews every turn-window `once` for its agent.
    fn renew(&mut self, agent: &str) {
        for ((fired_agent, _), fired) in &mut self.fired {
            if fired_agent == agent && fired.history == History::Turn {
                fired.spent = false;
            }
        }
    }
}

impl Capability for Rules {
    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        serde_json::to_value(SavedRef {
            histories: &self.histories,
            fired: self
                .fired
                .iter()
                .map(|(key, fired)| (key, *fired))
                .collect(),
        })
        .ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        let Ok(saved) = serde_json::from_value::<Saved>(state) else {
            return;
        };

        self.histories = Histories::restore(saved.histories);
        self.fired = saved.fired.into_iter().take(MAX_FIRED).collect();
    }

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        self.active = None;

        let agent = signal.agent_id.as_str();

        match &signal.kind {
            SignalKind::UserMessage { .. } => {
                self.histories.user_message(agent);
                self.renew(agent);
                Plan::Skip
            }
            SignalKind::IntegrationEvent { source, kind, .. } => {
                self.histories.hook(agent, format!("{source}:{kind}"));
                Plan::Skip
            }
            SignalKind::ToolResult {
                tool, workspace, ..
            } => {
                self.histories.tool(agent, workspace, tool);
                self.start(signal, Trigger::ToolResult, workspace, Some(tool))
            }
            SignalKind::TurnEnd { workspace, .. } => {
                self.start(signal, Trigger::TurnEnd, workspace, None)
            }
            _ => Plan::Skip,
        }
    }

    fn decide(&mut self, _: &Signal, _: Option<&[Answer]>) -> Vec<Effect> {
        Vec::new()
    }

    /// Round 1 judges every admitted rule's first step; round 2 the second
    /// step of those that held. A failed judgment confirms nothing new, and
    /// rules confirmed earlier still deliver.
    fn advance(&mut self, signal: &Signal, answers: Option<&[Answer]>, round: usize) -> PipeStep {
        let Some(mut active) = self.active.take() else {
            return PipeStep::Done(Vec::new());
        };
        let step = round.saturating_sub(1);
        let indices: Vec<usize> = if step == 0 {
            (0..active.rules.len()).collect()
        } else {
            std::mem::take(&mut active.next)
        };
        let mut next = Vec::new();

        for index in indices {
            let Some(rule) = active.rules.get(index) else {
                continue;
            };

            if !confirmed(rule, step, answers) {
                continue;
            }
            if step.saturating_add(1) < rule.steps.len() {
                next.push(index);
            } else {
                active.confirmed.push(index);
            }
        }

        let questions: Vec<Question> = next
            .iter()
            .filter_map(|index| active.rules.get(*index))
            .filter_map(|rule| question(rule, step.saturating_add(1), &signal.kind))
            .collect();

        if questions.is_empty() {
            return PipeStep::Done(self.deliver(signal, active));
        }

        active.next = next;
        self.active = Some(active);

        PipeStep::Next(questions)
    }
}

/// Whether Jev confirmed `rule`'s step at `index`.
fn confirmed(rule: &Rule, index: usize, answers: Option<&[Answer]>) -> bool {
    let (Some(step), Some(answers)) = (rule.steps.get(index), answers) else {
        return false;
    };
    let id = question_id(rule, step);

    answers.iter().any(|answer| {
        answer.id == id
            && matches!(answer.value, AnswerValue::Noul(p) if p >= step.yes_at_or_above)
            && answer.effective_confidence() >= step.minimum_confidence
    })
}

fn question_id(rule: &Rule, step: &Step) -> String {
    format!("{}/{}", rule.id, step.id)
}

/// A step's question with what it is about: the call a tool-result rule
/// judges, or the user's latest request at a turn end.
fn question(rule: &Rule, index: usize, kind: &SignalKind) -> Option<Question> {
    let step = rule.steps.get(index)?;
    let instructions = match kind {
        SignalKind::ToolResult {
            tool, ok, input, ..
        } => {
            let outcome = if *ok { "succeeded" } else { "failed" };

            // The call comes first so the rule can ask about "this call".
            format!(
                "The agent's latest {tool} call {outcome} with input: {input}\n{}\nA call the \
                 user explicitly asked for, exactly as asked, does not count.",
                step.question
            )
        }
        SignalKind::TurnEnd { user_request, .. } if !user_request.trim().is_empty() => format!(
            "{} Latest user request: {user_request}. Respect explicit user instructions.",
            step.question
        ),
        _ => step.question.clone(),
    };

    Some(Question {
        id: question_id(rule, step),
        instructions,
        kind: QuestionKind::Noul,
    })
}
