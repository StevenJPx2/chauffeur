//! Monitors: after a tool call names a pull request the agent opened, a Jira
//! issue it worked on, or a Slack thread, System One judges whether it is the
//! agent's own work to follow. A confirmed one gets a monitor from the
//! integration (sourcefed), unless one already watches it, and the agent is
//! told, so it does not set one up itself. An unreachable integration or a
//! failed judgment creates nothing. Run it as
//! `Judging::new(FollowWork::new(monitors))`, with your bar as
//! `FollowWork::new(monitors).with_config(MonitorsConfig::load(path)?)`.

mod detect;
mod watch;

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use chauffeur_core::judge::strategy::{self, Candidate};
use chauffeur_core::{
    Delivery, Effect, Judge, Judged, Question, QuestionKind, Signal, SignalKind, Situation,
    Template, Threshold, load_layered,
};
use serde::Deserialize;

pub use detect::candidates;
pub use watch::{Monitor, Monitors, Watch};

pub const ID: &str = "monitors";
const MAX_AGENTS: usize = 256;
const MAX_JUDGED: usize = 64;
/// The shipped bar (`skills/config/monitors.json`), compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/monitors.json");

/// When a candidate is followed, and how the question and the notice read.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MonitorsConfig {
    /// A confident yes at or above this bar that the work is the session's
    /// own gets a monitor.
    pub follow: Threshold,
    /// The wording of the question to System One and the notice to the agent.
    pub texts: MonitorsTexts,
}

/// The question System One answers about a candidate, and what the agent is told.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MonitorsTexts {
    /// Whether the session should follow it: `{tool}`, `{input}`, `{watch}`.
    pub question: Template,
    /// The notice to the agent once it is followed: `{watch}`.
    pub followed: Template,
}

impl MonitorsTexts {
    fn checked(self) -> Result<Self, String> {
        self.question
            .check("texts.question", &["tool", "input", "watch"])?;
        self.followed.check("texts.followed", &["watch"])?;

        Ok(self)
    }
}

impl MonitorsConfig {
    /// The shipped config overlaid by your `monitors.json` at `path`.
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

impl Default for MonitorsConfig {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
            .expect("shipped monitors defaults are valid")
    }
}

pub struct FollowWork {
    monitors: Arc<dyn Monitors>,
    config: MonitorsConfig,
    /// Watches already judged per agent, so a call is asked about once.
    judged: HashMap<String, HashSet<Watch>>,
}

impl FollowWork {
    /// Follow work through `monitors` at the shipped bar.
    #[must_use]
    pub fn new(monitors: Arc<dyn Monitors>) -> Self {
        Self {
            monitors,
            config: MonitorsConfig::default(),
            judged: HashMap::new(),
        }
    }

    /// This capability at `config`'s bar.
    #[must_use]
    pub fn with_config(mut self, config: MonitorsConfig) -> Self {
        self.config = config;
        self
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
                text: Some(
                    self.config
                        .texts
                        .followed
                        .render(&[("watch", &watch.describe())]),
                ),
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

/// What a judged call decided: every watch asked about, and the question
/// indices of those confirmed.
pub struct Outcome {
    asked: Vec<Watch>,
    confirmed: Vec<usize>,
}

impl Judged for FollowWork {
    /// `None` when the call failed.
    type Verdict = Option<Outcome>;

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

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<Option<Outcome>>> {
        let SignalKind::ToolResult {
            tool,
            ok: true,
            input,
            evidence,
            ..
        } = &signal.kind
        else {
            return None;
        };
        let fresh: Vec<Watch> = candidates(tool, input, evidence)
            .into_iter()
            .filter(|watch| !self.judged(&signal.agent_id, watch))
            .collect();

        if fresh.is_empty() {
            return None;
        }

        // An unreachable integration cannot follow anything, so nothing is asked.
        let existing = self.monitors.list(&signal.agent_id).ok()?;
        let watched: Vec<Watch> = existing
            .into_iter()
            .filter_map(|monitor| monitor.watch)
            .collect();
        let (already, new): (Vec<Watch>, Vec<Watch>) =
            fresh.into_iter().partition(|watch| watched.contains(watch));

        self.record(&signal.agent_id, &already);

        if new.is_empty() {
            return None;
        }

        let candidates: Vec<Candidate<usize>> = new
            .iter()
            .enumerate()
            .map(|(index, watch)| Candidate {
                key: index,
                question: question(&self.config.texts.question, index, watch, (tool, input)),
                rule: self.config.follow.yes(),
            })
            .collect();

        Some(
            strategy::fan_out(candidates)
                .unless_failed()
                .map(|confirmed| {
                    confirmed.map(|confirmed| Outcome {
                        asked: new,
                        confirmed,
                    })
                }),
        )
    }

    /// Follow the confirmed watches in question order. Every asked watch is
    /// judged; a failed call follows nothing and may be judged again.
    fn act(&mut self, signal: &Signal, outcome: Option<Outcome>) -> Vec<Effect> {
        let Some(Outcome {
            asked,
            mut confirmed,
        }) = outcome
        else {
            return Vec::new();
        };

        self.record(&signal.agent_id, &asked);
        confirmed.sort_unstable();

        confirmed
            .iter()
            .filter_map(|index| asked.get(*index))
            .filter_map(|watch| self.follow(&signal.agent_id, watch))
            .collect()
    }
}

fn question(template: &Template, index: usize, watch: &Watch, call: (&str, &str)) -> Question {
    let (tool, input) = call;

    Question {
        id: format!("follow/{index}"),
        instructions: template.render(&[
            ("tool", tool),
            ("input", input),
            ("watch", &watch.describe()),
        ]),
        kind: QuestionKind::Noul,
    }
}
