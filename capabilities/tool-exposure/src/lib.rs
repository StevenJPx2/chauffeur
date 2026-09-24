//! Tool exposure, by tool group (the ID prefix before the first `_`, such as
//! `browser` for `browser_tabs_list`). At a context's first user message,
//! before any prompt cache exists, System One judges whether each group will
//! be needed, and confidently unneeded groups are hidden. On later messages
//! the host sends the hidden tools, and System One may bring a group back,
//! which re-reads the conversation once. Base tools are never hidden, and the
//! effects name tools, so a tool the engine never judged is never removed.
//! Code Mode tools reach the model through `execute`, whose catalog shows each
//! namespace only in part; a namespace the request needs is surfaced by an
//! appended note instead.

mod code_mode;

use std::collections::HashSet;
use std::path::Path;

use chauffeur_core::{
    Answer, AnswerValue, Capability, CatalogEntry, Effect, Plan, Question, QuestionKind, Signal,
    SignalKind, Situation, load_config,
};

use code_mode::Surfaced;
use serde::Deserialize;

pub const ID: &str = "tool-exposure";
/// OpenCode's built-in tools, never hidden.
pub const DEFAULT_BASE: &[&str] = &[
    "read",
    "edit",
    "patch",
    "write",
    "shell",
    "grep",
    "glob",
    "question",
    "subagent",
    "webfetch",
    "websearch",
];
/// Chauffeur owns skill loading, so the host's skill loader is never exposed.
pub const NEVER_EXPOSED: &[&str] = &["skill"];
/// Hide a group only when P(needed) is at most this…
pub const HIDE_AT_OR_BELOW: f32 = 0.3;
/// …bring a hidden group back only when P(needed) is at least this…
pub const REVEAL_AT_OR_ABOVE: f32 = 0.7;
/// …and in both cases the answer is at least this confident.
pub const MIN_CONFIDENCE: f32 = 0.4;
/// Groups judged per message, in host order.
pub const MAX_GROUPS: usize = 64;
const MAX_BASE: usize = 64;
const MAX_LISTED_TOOLS: usize = 6;

/// A tool's group: its ID up to the first `_`.
#[must_use]
pub fn group(tool_id: &str) -> &str {
    tool_id
        .split_once('_')
        .map_or(tool_id, |(prefix, _)| prefix)
}

/// Strict JSON config. `base` replaces [`DEFAULT_BASE`].
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolExposureConfig {
    #[serde(default)]
    pub base: Option<Vec<String>>,
}

impl ToolExposureConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        load_config::<Self>(path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    /// The config, if within bounds.
    pub fn checked(self) -> Result<Self, String> {
        if self.base.as_ref().is_some_and(|base| base.len() > MAX_BASE) {
            return Err(format!("more than {MAX_BASE} base tools"));
        }

        Ok(self)
    }
}

pub struct ToolExposure {
    base: HashSet<String>,
    surfaced: Surfaced,
}

/// One group of judgeable tools, in host order.
struct Group<'a> {
    name: &'a str,
    tools: Vec<&'a CatalogEntry>,
}

impl ToolExposure {
    #[must_use]
    pub fn new(config: ToolExposureConfig) -> Self {
        let base = config.base.unwrap_or_else(|| {
            DEFAULT_BASE
                .iter()
                .map(|tool| (*tool).to_string())
                .collect()
        });

        Self {
            base: base.into_iter().collect(),
            surfaced: Surfaced::default(),
        }
    }

    fn judgeable(&self, tool: &CatalogEntry) -> bool {
        !self.base.contains(&tool.id) && !NEVER_EXPOSED.contains(&tool.id.as_str())
    }

    /// Judgeable tools grouped by prefix, groups in first-seen order.
    fn groups<'a>(&self, tools: &'a [CatalogEntry]) -> Vec<Group<'a>> {
        let mut seen = HashSet::new();
        let mut groups: Vec<Group<'a>> = Vec::new();

        for tool in tools.iter().filter(|tool| self.judgeable(tool)) {
            if !seen.insert(tool.id.as_str()) {
                continue;
            }

            let name = group(&tool.id);

            if let Some(existing) = groups.iter_mut().find(|existing| existing.name == name) {
                existing.tools.push(tool);
            } else if groups.len() < MAX_GROUPS {
                groups.push(Group {
                    name,
                    tools: vec![tool],
                });
            }
        }

        groups
    }

    /// Never-exposed tools present, plus the tools of groups `hide` selects.
    fn hidden(&self, tools: &[CatalogEntry], hide: impl Fn(&str) -> bool) -> Vec<String> {
        let chosen: HashSet<&str> = self
            .groups(tools)
            .iter()
            .filter(|group| hide(group.name))
            .flat_map(|group| group.tools.iter().map(|tool| tool.id.as_str()))
            .collect();
        let mut seen = HashSet::new();

        tools
            .iter()
            .filter(|tool| {
                NEVER_EXPOSED.contains(&tool.id.as_str()) || chosen.contains(tool.id.as_str())
            })
            .filter(|tool| seen.insert(tool.id.clone()))
            .map(|tool| tool.id.clone())
            .collect()
    }

    /// Tools of the hidden groups `reveal` selects.
    fn revealed(&self, tools: &[CatalogEntry], reveal: impl Fn(&str) -> bool) -> Vec<String> {
        self.groups(tools)
            .iter()
            .filter(|group| reveal(group.name))
            .flat_map(|group| group.tools.iter().map(|tool| tool.id.clone()))
            .collect()
    }
}

fn effect(agent_id: &str, tools: Vec<String>, reveal: bool) -> Vec<Effect> {
    if tools.is_empty() {
        return Vec::new();
    }

    let (hide, reveal) = if reveal {
        (Vec::new(), tools)
    } else {
        (tools, Vec::new())
    };

    vec![Effect::Tools {
        agent_id: agent_id.to_string(),
        hide,
        reveal,
    }]
}

impl Capability for ToolExposure {
    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        Some(self.surfaced.save())
    }

    fn load(&mut self, state: serde_json::Value) {
        self.surfaced.load(state);
    }

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        let SignalKind::UserMessage {
            first_in_context,
            tools,
            code_mode,
            ..
        } = &signal.kind
        else {
            return Plan::Skip;
        };

        if *first_in_context {
            self.surfaced.reset(&signal.agent_id);
        }

        let groups = self.groups(tools);
        let surface: Vec<Question> = self
            .surfaced
            .pending(&signal.agent_id, code_mode)
            .into_iter()
            .take(MAX_GROUPS)
            .map(code_mode::question)
            .collect();

        if groups.is_empty() && surface.is_empty() {
            let never = if *first_in_context {
                self.hidden(tools, |_| false)
            } else {
                Vec::new()
            };

            return match effect(&signal.agent_id, never, false) {
                effects if effects.is_empty() => Plan::Skip,
                effects => Plan::Settled(effects),
            };
        }

        let ask = if *first_in_context {
            hide_question
        } else {
            reveal_question
        };

        Plan::Ask(groups.iter().map(ask).chain(surface).collect())
    }

    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        let (
            SignalKind::UserMessage {
                first_in_context,
                tools,
                code_mode,
                ..
            },
            Some(answers),
        ) = (&signal.kind, answers)
        else {
            // Fail open: nothing changes, so the host keeps its current tools.
            return Vec::new();
        };
        let judged =
            |name: &str, accept: fn(f32) -> bool| {
                answers.iter().find(|answer| answer.id == name).is_some_and(|answer| {
                matches!(answer.value, AnswerValue::Noul(probability) if accept(probability))
                    && answer.effective_confidence() >= MIN_CONFIDENCE
            })
            };

        let mut effects = if *first_in_context {
            let hide = self.hidden(tools, |name| judged(name, |p| p <= HIDE_AT_OR_BELOW));

            effect(&signal.agent_id, hide, false)
        } else {
            let reveal = self.revealed(tools, |name| judged(name, |p| p >= REVEAL_AT_OR_ABOVE));

            effect(&signal.agent_id, reveal, true)
        };
        let chosen: Vec<_> = self
            .surfaced
            .pending(&signal.agent_id, code_mode)
            .into_iter()
            .take(MAX_GROUPS)
            .filter(|namespace| {
                judged(&code_mode::question_id(&namespace.name), |p| {
                    p >= REVEAL_AT_OR_ABOVE
                })
            })
            .collect();

        effects.extend(self.surfaced.surface(&signal.agent_id, &chosen));
        effects
    }
}

fn listing(group: &Group<'_>) -> String {
    let mut listed: Vec<String> = group
        .tools
        .iter()
        .take(MAX_LISTED_TOOLS)
        .map(|tool| format!("{} ({})", tool.id, tool.description))
        .collect();

    if group.tools.len() > MAX_LISTED_TOOLS {
        listed.push(format!("and {} more", group.tools.len() - MAX_LISTED_TOOLS));
    }

    listed.join("; ")
}

fn hide_question(group: &Group<'_>) -> Question {
    Question {
        id: group.name.to_string(),
        instructions: format!(
            "Will the coding agent need any of the \"{}\" tools to carry out the user's task? \
             Tools: {}",
            group.name,
            listing(group)
        ),
        kind: QuestionKind::Noul,
    }
}

fn reveal_question(group: &Group<'_>) -> Question {
    Question {
        id: group.name.to_string(),
        instructions: format!(
            "The \"{}\" tools were hidden from the coding agent earlier in this conversation. \
             Does the user's latest request need them now? Showing them again re-reads the whole \
             conversation once, so answer yes only when they are clearly needed. Tools: {}",
            group.name,
            listing(group)
        ),
        kind: QuestionKind::Noul,
    }
}
