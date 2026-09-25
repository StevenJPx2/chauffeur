//! Monitors: after a tool call names a pull request the agent opened, a Jira
//! issue it worked on, or a Slack thread, System One judges whether it is the
//! agent's own work to follow. A confirmed one gets a monitor from the
//! integration (sourcefed), unless one already watches it, and the agent is
//! told, so it does not set one up itself. An unreachable integration or a
//! failed judgment creates nothing.

mod detect;
mod watch;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chauffeur_core::{
    Answer, AnswerValue, Capability, Delivery, Effect, Plan, Question, QuestionKind, Signal,
    SignalKind, Situation,
};

pub use detect::candidates;
pub use watch::{Monitor, Monitors, Watch};

pub const ID: &str = "monitors";
/// Follow when P(own work) is at least this…
pub const FOLLOW_AT_OR_ABOVE: f32 = 0.7;
/// …and the answer is at least this confident.
pub const MIN_CONFIDENCE: f32 = 0.4;
const MAX_AGENTS: usize = 256;
const MAX_JUDGED: usize = 64;

pub struct FollowWork {
    monitors: Arc<dyn Monitors>,
    /// Watches already judged per agent, so a call is asked about once.
    judged: HashMap<String, HashSet<Watch>>,
    /// The watches the current signal asks about, in question order.
    pending: Vec<Watch>,
}

impl FollowWork {
    #[must_use]
    pub fn new(monitors: Arc<dyn Monitors>) -> Self {
        Self {
            monitors,
            judged: HashMap::new(),
            pending: Vec::new(),
        }
    }

    fn judged(&self, agent: &str, watch: &Watch) -> bool {
        self.judged
            .get(agent)
            .is_some_and(|judged| judged.contains(watch))
    }

    fn record(&mut self, agent: &str, watches: &[Watch]) {
        if !self.judged.contains_key(agent) && self.judged.len() >= MAX_AGENTS {
            self.judged.clear();
        }

        let judged = self.judged.entry(agent.to_string()).or_default();

        if judged.len() + watches.len() > MAX_JUDGED {
            judged.clear();
        }

        judged.extend(watches.iter().cloned());
    }

    /// Create the monitor and tell the agent; `None` when the integration fails.
    fn follow(&self, agent: &str, watch: &Watch) -> Option<Effect> {
        match self.monitors.create(agent, watch) {
            Ok(_) => Some(Effect::Context {
                agent_id: agent.to_string(),
                delivery: Delivery::Steer,
                label: format!("watching {}", watch.name()),
                skills: Vec::new(),
                text: Some(format!(
                    "Chauffeur: sourcefed now watches {} for this session; new reviews, CI \
                     results, and comments will arrive here. Do not create another monitor for it.",
                    watch.describe()
                )),
            }),
            Err(error) => {
                eprintln!(
                    "chauffeur: monitor for {} not created: {error}",
                    watch.name()
                );
                None
            }
        }
    }
}

impl Capability for FollowWork {
    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        serde_json::to_value(&self.judged).ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        if let Ok(judged) = serde_json::from_value::<HashMap<String, HashSet<Watch>>>(state) {
            self.judged = judged.into_iter().take(MAX_AGENTS).collect();
        }
    }

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        self.pending.clear();

        let SignalKind::ToolResult {
            tool,
            ok: true,
            input,
            evidence,
            ..
        } = &signal.kind
        else {
            return Plan::Skip;
        };
        let fresh: Vec<Watch> = candidates(tool, input, evidence)
            .into_iter()
            .filter(|watch| !self.judged(&signal.agent_id, watch))
            .collect();

        if fresh.is_empty() {
            return Plan::Skip;
        }

        // An unreachable integration cannot follow anything, so nothing is asked.
        let Ok(existing) = self.monitors.list(&signal.agent_id) else {
            return Plan::Skip;
        };
        let watched: Vec<Watch> = existing
            .into_iter()
            .filter_map(|monitor| monitor.watch)
            .collect();
        let (already, new): (Vec<Watch>, Vec<Watch>) =
            fresh.into_iter().partition(|watch| watched.contains(watch));

        self.record(&signal.agent_id, &already);

        if new.is_empty() {
            return Plan::Skip;
        }

        let questions = new
            .iter()
            .enumerate()
            .map(|(index, watch)| question(index, watch, tool, input))
            .collect();

        self.pending = new;

        Plan::Ask(questions)
    }

    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        // A failed judgment follows nothing, and the call may be judged again.
        let Some(answers) = answers else {
            return Vec::new();
        };
        let pending = std::mem::take(&mut self.pending);

        self.record(&signal.agent_id, &pending);

        pending
            .iter()
            .enumerate()
            .filter(|(index, _)| confirmed(answers, *index))
            .filter_map(|(_, watch)| self.follow(&signal.agent_id, watch))
            .collect()
    }
}

fn question_id(index: usize) -> String {
    format!("follow/{index}")
}

fn confirmed(answers: &[Answer], index: usize) -> bool {
    let id = question_id(index);

    answers.iter().any(|answer| {
        answer.id == id
            && matches!(answer.value, AnswerValue::Noul(p) if p >= FOLLOW_AT_OR_ABOVE)
            && answer.effective_confidence() >= MIN_CONFIDENCE
    })
}

fn question(index: usize, watch: &Watch, tool: &str, input: &str) -> Question {
    Question {
        id: question_id(index),
        instructions: format!(
            "The agent's latest {tool} call, with input {input}, involves {}. Is it the coding \
             session's own work, which the session should keep following so new reviews, CI \
             results, and comments reach the agent? Yes when the agent created it or is working \
             on it for the user's task; no when it only looked it up or it is someone else's work.",
            watch.describe()
        ),
        kind: QuestionKind::Noul,
    }
}
