//! Event gate: integrations such as sourcefed ask before an event reaches the
//! agent. System One judges whether the event needs the agent to act; only a
//! confident "no" withholds it, so a failed or unsure judgment delivers. With
//! the integration's monitors, the question also says what the event's
//! monitor follows. Run it as `Judging::new(EventGate::default())`, with
//! your bar as `EventGate::default().with_config(EventGateConfig::load(path)?)`.

use std::path::Path;
use std::sync::Arc;

use chauffeur_capability_monitors::Monitors;
use chauffeur_core::judge::strategy;
use chauffeur_core::{
    Effect, Judge, Judged, Question, QuestionKind, Signal, SignalKind, Situation, Template,
    Threshold, load_layered,
};
use serde::Deserialize;

pub const ID: &str = "event-gate";
const QUESTION: &str = "show";
const MAX_BODY_CHARS: usize = 600;
/// The shipped bar (`skills/config/event-gate.json`), compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/event-gate.json");

/// When an event is withheld, and how the question about it reads.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventGateConfig {
    /// A confident no at or below this bar withholds the event; anything
    /// else delivers it.
    pub withhold: Threshold,
    /// The wording of the question to System One.
    pub texts: EventGateTexts,
}

/// The question System One answers about an event, and its fragments.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventGateTexts {
    /// Whether to show the event: `{source}`, `{kind}`, `{summary}`, `{body}`,
    /// `{origin}`, `{marked}`.
    pub question: Template,
    /// `{body}` for an event with a body: `{body}`.
    pub body: Template,
    /// `{origin}` for an event whose monitor is known: `{watch}`.
    pub origin: Template,
    /// `{marked}` for an event sourcefed marks actionable.
    pub marked_actionable: Template,
    /// `{marked}` for an event sourcefed marks informational.
    pub marked_informational: Template,
}

impl EventGateTexts {
    fn checked(self) -> Result<Self, String> {
        for (field, template, allowed) in [
            (
                "texts.question",
                &self.question,
                &["source", "kind", "summary", "body", "origin", "marked"][..],
            ),
            ("texts.body", &self.body, &["body"][..]),
            ("texts.origin", &self.origin, &["watch"][..]),
            ("texts.marked_actionable", &self.marked_actionable, &[][..]),
            (
                "texts.marked_informational",
                &self.marked_informational,
                &[][..],
            ),
        ] {
            template.check(field, allowed)?;
        }

        Ok(self)
    }
}

impl EventGateConfig {
    /// The shipped config overlaid by your `event-gate.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, a bar is outside `[0, 1]`, or a
    /// text is empty or names a placeholder it may not use.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered::<Self>(SHIPPED, path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    fn checked(mut self) -> Result<Self, String> {
        self.texts = self.texts.checked()?;

        Ok(self)
    }
}

impl Default for EventGateConfig {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
            .expect("shipped event-gate defaults are valid")
    }
}

#[derive(Default)]
pub struct EventGate {
    monitors: Option<Arc<dyn Monitors>>,
    config: EventGateConfig,
}

impl EventGate {
    /// A gate that looks up the monitor behind each event, at the shipped bar.
    #[must_use]
    pub fn with_monitors(monitors: Arc<dyn Monitors>) -> Self {
        Self {
            monitors: Some(monitors),
            config: EventGateConfig::default(),
        }
    }

    /// This gate at `config`'s bar.
    #[must_use]
    pub fn with_config(mut self, config: EventGateConfig) -> Self {
        self.config = config;
        self
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
            self.config
                .texts
                .origin
                .render(&[("watch", &watch.describe())])
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
        let texts = &self.config.texts;
        let body = if body.is_empty() {
            String::new()
        } else {
            texts.body.render(&[("body", &body)])
        };
        let marked = if *actionable {
            &texts.marked_actionable
        } else {
            &texts.marked_informational
        }
        .render(&[]);
        let question = Question {
            id: QUESTION.into(),
            instructions: texts.question.render(&[
                ("source", source),
                ("kind", kind),
                ("summary", summary),
                ("body", &body),
                ("origin", &origin),
                ("marked", &marked),
            ]),
            kind: QuestionKind::Noul,
        };

        Some(strategy::single(question, self.config.withhold.no()))
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
