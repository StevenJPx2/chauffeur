//! Event gate: integrations such as sourcefed ask before an event reaches the
//! agent. System One judges whether the event needs the agent to act; only a
//! confident "no" withholds it, so a failed or unsure judgment delivers. With
//! the integration's monitors, the question also says what the event's
//! monitor follows. Run it as `Judging::new(EventGate::default())`.

use std::sync::Arc;

use chauffeur_capability_monitors::Monitors;
use chauffeur_core::judge::strategy;
use chauffeur_core::{
    Effect, Judge, Judged, Question, QuestionKind, Rule, Signal, SignalKind, Situation,
};

pub const ID: &str = "event-gate";
/// Withhold only when P(show) is at most this…
pub const WITHHOLD_AT_OR_BELOW: f32 = 0.3;
/// …and the answer is at least this confident.
pub const MIN_CONFIDENCE: f32 = 0.4;
const QUESTION: &str = "show";
const MAX_BODY_CHARS: usize = 600;

#[derive(Default)]
pub struct EventGate {
    monitors: Option<Arc<dyn Monitors>>,
}

impl EventGate {
    /// A gate that looks up the monitor behind each event.
    #[must_use]
    pub fn with_monitors(monitors: Arc<dyn Monitors>) -> Self {
        Self {
            monitors: Some(monitors),
        }
    }

    /// What the event's monitor follows, as a sentence; empty when unknown.
    fn origin(&self, agent: &str, monitor: &str) -> String {
        let Some(monitors) = self.monitors.as_ref().filter(|_| !monitor.is_empty()) else {
            return String::new();
        };
        let watch = monitors
            .list(agent)
            .ok()
            .and_then(|all| all.into_iter().find(|candidate| candidate.id == monitor))
            .and_then(|found| found.watch);

        watch.map_or_else(String::new, |watch| {
            format!(
                "\nIt comes from this session's monitor on {}.",
                watch.describe()
            )
        })
    }
}

impl Judged for EventGate {
    /// Whether to withhold the event.
    type Verdict = bool;

    fn id(&self) -> &str {
        ID
    }

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<bool>> {
        let SignalKind::IntegrationEvent {
            source,
            kind,
            summary,
            body,
            actionable,
            monitor,
        } = &signal.kind
        else {
            return None;
        };
        let origin = self.origin(&signal.agent_id, monitor);
        let body: String = body.chars().take(MAX_BODY_CHARS).collect();
        let body = if body.is_empty() {
            String::new()
        } else {
            format!("\n{body}")
        };
        let marked = if *actionable {
            "actionable"
        } else {
            "informational"
        };
        let question = Question {
            id: QUESTION.into(),
            instructions: format!(
                "A {source} {kind} event arrived for the coding agent's session: {summary}{body}{origin}\n\
                 sourcefed marks it {marked}. Should the agent be shown this event now? Show \
                 events that need the agent to do something: CI failures, requested changes, \
                 questions or requests addressed to it, merge conflicts, and a merged pull \
                 request with follow-up work. Withhold noise: bot comments, approvals, status or \
                 assignee changes nobody asked it to act on, thanks, and anything it has already \
                 handled."
            ),
            kind: QuestionKind::Noul,
        };

        Some(strategy::single(
            question,
            Rule::no(WITHHOLD_AT_OR_BELOW, MIN_CONFIDENCE),
        ))
    }

    fn act(&mut self, signal: &Signal, withhold: bool) -> Vec<Effect> {
        if !withhold {
            return Vec::new();
        }

        vec![Effect::Gate {
            agent_id: signal.agent_id.clone(),
            deliver: false,
        }]
    }
}
