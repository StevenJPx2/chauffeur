//! Tool exposure, by tool group (the ID prefix before the first `_`, such as
//! `browser` for `browser_tabs_list`). At a context's first user message,
//! before any prompt cache exists, System One judges whether each group will
//! be needed, and confidently unneeded groups are hidden. On later messages
//! the host sends the hidden tools, and System One may bring a group back,
//! which re-reads the conversation once. Base tools are never hidden, and the
//! effects name tools, so a tool the engine never judged is never removed.
//! Code Mode tools reach the model through `execute`, whose catalog shows each
//! namespace only in part; a namespace the request needs is surfaced by an
//! appended note instead. When the agent asks for a tool itself, System One
//! judges every hidden group and namespace, and each that serves it is
//! granted. Run it as `Judging::new(ToolExposure::new(config))`.

mod code_mode;
mod config;
mod request;

use std::collections::HashSet;

use chauffeur_core::judge::strategy::{self, Candidate};
use chauffeur_core::{
    CatalogEntry, ChoiceOption, Delivery, Effect, Judge, Judged, Question, QuestionKind, Rule,
    Signal, SignalKind, Situation,
};

use code_mode::Surfaced;

pub use config::ToolExposureConfig;

pub const ID: &str = "tool-exposure";
/// Groups judged per message, in host order.
pub const MAX_GROUPS: usize = 64;
/// The recovery confirmation that reveals the chosen tool.
const REVEAL_NEEDED: &str = "reveal_needed";

/// A tool's group: its ID up to the first `_`.
#[must_use]
pub fn group(tool_id: &str) -> &str {
    tool_id
        .split_once('_')
        .map_or(tool_id, |(prefix, _)| prefix)
}

/// What a finished judge found, by signal kind.
pub enum Verdict {
    /// A user message: the groups whose rule held, hidden at the first
    /// message and revealed later, and the Code Mode namespaces to surface.
    /// `heard` is false when nothing was answered, so nothing changes.
    Message {
        first: bool,
        heard: bool,
        groups: Vec<String>,
        namespaces: Vec<String>,
    },
    /// A missing-tool result: the hidden direct tool to reveal, if any.
    Recover(Option<String>),
    /// The agent's request: everything that serves it.
    Request(Vec<Grant>),
}

/// What the agent's request may be granted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Grant {
    Group(String),
    Namespace(String),
}

pub struct ToolExposure {
    config: ToolExposureConfig,
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
        Self {
            base: config.base.iter().cloned().collect(),
            config,
            surfaced: Surfaced::default(),
        }
    }

    fn never_exposed(&self, tool: &CatalogEntry) -> bool {
        self.config.never_exposed.contains(&tool.id)
    }

    fn judgeable(&self, tool: &CatalogEntry) -> bool {
        !self.base.contains(&tool.id) && !self.never_exposed(tool)
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
            .filter(|tool| self.never_exposed(tool) || chosen.contains(tool.id.as_str()))
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

    /// At a user message: every group, hidden on a confident no at the first
    /// message and revealed on a confident yes later, beside every Code Mode
    /// namespace not yet surfaced, all in one round.
    fn message(&mut self, signal: &Signal) -> Option<Judge<Verdict>> {
        let SignalKind::UserMessage {
            first_in_context,
            tools,
            code_mode,
            ..
        } = &signal.kind
        else {
            return None;
        };
        let first = *first_in_context;

        if first {
            self.surfaced.reset(&signal.agent_id);
        }

        // A group the task confidently will not need is hidden; a hidden
        // group or namespace clearly needed is brought in.
        let needed = self.config.reveal.yes();
        let (ask, rule): (fn(&Group<'_>, usize) -> Question, Rule) = if first {
            (hide_question, self.config.hide.no())
        } else {
            (reveal_question, needed)
        };
        let groups: Vec<Candidate<String>> = self
            .groups(tools)
            .iter()
            .map(|group| Candidate {
                key: group.name.to_string(),
                question: ask(group, self.config.listed_tools),
                rule,
            })
            .collect();
        let namespaces: Vec<Candidate<String>> = self
            .surfaced
            .pending(&signal.agent_id, code_mode)
            .into_iter()
            .take(MAX_GROUPS)
            .map(|namespace| Candidate {
                key: namespace.name.clone(),
                question: code_mode::question(namespace),
                rule: needed,
            })
            .collect();

        if groups.is_empty() && namespaces.is_empty() {
            return Some(Judge::done(Verdict::Message {
                first,
                heard: true,
                groups: Vec::new(),
                namespaces: Vec::new(),
            }));
        }

        // Groups and namespaces in one round; a failed call changes nothing.
        Some(
            strategy::fan_out(groups)
                .zip(strategy::fan_out(namespaces))
                .unless_failed()
                .map(move |judged| {
                    let heard = judged.is_some();
                    let (groups, namespaces) = judged.unwrap_or_default();

                    Verdict::Message {
                        first,
                        heard,
                        groups,
                        namespaces,
                    }
                }),
        )
    }

    fn recovery_candidates<'a>(&self, signal: &'a Signal) -> Vec<&'a CatalogEntry> {
        let SignalKind::ToolResult { candidates, .. } = &signal.kind else {
            return Vec::new();
        };
        let mut seen = HashSet::new();

        candidates
            .iter()
            .filter(|tool| self.judgeable(tool) && seen.insert(tool.id.as_str()))
            .take(self.config.recovery_candidates)
            .collect()
    }

    /// After a missing-tool result: which hidden direct tool fits, then,
    /// a round later, whether the agent clearly needs it.
    fn recover(&self, signal: &Signal) -> Option<Judge<Verdict>> {
        let SignalKind::ToolResult {
            user_request,
            evidence,
            ..
        } = &signal.kind
        else {
            return None;
        };
        let candidates: Vec<(String, String)> = self
            .recovery_candidates(signal)
            .iter()
            .map(|tool| (tool.id.clone(), tool.description.clone()))
            .collect();

        if candidates.is_empty() {
            return None;
        }

        let choose = choose_question(&candidates, user_request, evidence);
        let (user_request, evidence) = (user_request.clone(), evidence.clone());
        let pick = self.config.pick_confidence.pick();

        Some(
            Judge::ask(choose, move |answer| pick.chosen(answer)).then(move |chosen| {
                let Some((tool, description)) =
                    chosen.and_then(|chosen| candidates.into_iter().find(|(id, _)| *id == chosen))
                else {
                    return Judge::done(Verdict::Recover(None));
                };
                let confirm = confirm_question(&tool, &description, &user_request, &evidence);

                Judge::ask(confirm, move |answer| {
                    Verdict::Recover(
                        (pick.chosen(answer).as_deref() == Some(REVEAL_NEEDED)).then_some(tool),
                    )
                })
            }),
        )
    }

    /// At an agent's request: every hidden group and Code Mode namespace,
    /// judged against what it asked for in one round.
    fn request(&self, signal: &Signal) -> Option<Judge<Verdict>> {
        let SignalKind::AgentRequest {
            need,
            user_request,
            tools,
            code_mode,
        } = &signal.kind
        else {
            return None;
        };
        let groups: Vec<(String, String)> = self
            .groups(tools)
            .iter()
            .map(|group| {
                (
                    group.name.to_string(),
                    listing(group, self.config.listed_tools),
                )
            })
            .collect();
        let candidates = request::candidates(
            &groups,
            code_mode,
            need,
            user_request,
            self.config.reveal.yes(),
        );

        (!candidates.is_empty()).then(|| strategy::fan_out(candidates).map(Verdict::Request))
    }

    /// Hide or reveal the judged groups, and surface the judged namespaces
    /// on the user's message.
    fn message_effects(
        &mut self,
        signal: &Signal,
        first: bool,
        groups: &[String],
        namespaces: &[String],
    ) -> Vec<Effect> {
        let SignalKind::UserMessage {
            tools, code_mode, ..
        } = &signal.kind
        else {
            return Vec::new();
        };
        let judged = |name: &str| groups.iter().any(|group| group == name);
        let mut effects = if first {
            effect(&signal.agent_id, self.hidden(tools, judged), false)
        } else {
            effect(&signal.agent_id, self.revealed(tools, judged), true)
        };
        let chosen: Vec<_> = self
            .surfaced
            .pending(&signal.agent_id, code_mode)
            .into_iter()
            .filter(|namespace| namespaces.contains(&namespace.name))
            .collect();

        effects.extend(
            self.surfaced
                .surface(&signal.agent_id, &chosen, Delivery::Prompt),
        );

        effects
    }

    /// Reveal every granted group in one effect, and bring every granted
    /// namespace's matches into the running turn in one note.
    fn granted(&mut self, signal: &Signal, grants: &[Grant]) -> Vec<Effect> {
        let SignalKind::AgentRequest {
            tools, code_mode, ..
        } = &signal.kind
        else {
            return Vec::new();
        };
        let reveal = self.revealed(tools, |name| {
            grants
                .iter()
                .any(|grant| matches!(grant, Grant::Group(group) if group == name))
        });
        let mut effects = effect(&signal.agent_id, reveal, true);
        let chosen: Vec<_> = code_mode
            .iter()
            .filter(|namespace| {
                grants
                    .iter()
                    .any(|grant| matches!(grant, Grant::Namespace(name) if *name == namespace.name))
            })
            .collect();

        effects.extend(
            self.surfaced
                .surface(&signal.agent_id, &chosen, Delivery::Steer),
        );

        effects
    }
}

impl Judged for ToolExposure {
    type Verdict = Verdict;

    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        Some(self.surfaced.save())
    }

    fn load(&mut self, state: serde_json::Value) {
        self.surfaced.load(state);
    }

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<Verdict>> {
        match signal.kind {
            SignalKind::UserMessage { .. } => self.message(signal),
            SignalKind::ToolResult { .. } => self.recover(signal),
            SignalKind::AgentRequest { .. } => self.request(signal),
            _ => None,
        }
    }

    fn act(&mut self, signal: &Signal, verdict: Verdict) -> Vec<Effect> {
        match verdict {
            // Fail open: nothing changes, so the host keeps its current tools.
            Verdict::Message { heard: false, .. } => Vec::new(),
            Verdict::Message {
                first,
                groups,
                namespaces,
                ..
            } => self.message_effects(signal, first, &groups, &namespaces),
            Verdict::Recover(tool) => effect(&signal.agent_id, tool.into_iter().collect(), true),
            Verdict::Request(grants) => self.granted(signal, &grants),
        }
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

/// The group's first `max` tools, and how many more it has.
fn listing(group: &Group<'_>, max: usize) -> String {
    let mut listed: Vec<String> = group
        .tools
        .iter()
        .take(max)
        .map(|tool| format!("{} ({})", tool.id, tool.description))
        .collect();

    if group.tools.len() > max {
        listed.push(format!("and {} more", group.tools.len() - max));
    }

    listed.join("; ")
}

fn hide_question(group: &Group<'_>, listed: usize) -> Question {
    Question {
        id: group.name.to_string(),
        instructions: format!(
            "Will the coding agent need any of the \"{}\" tools to carry out the user's task? \
             Tools: {}",
            group.name,
            listing(group, listed)
        ),
        kind: QuestionKind::Noul,
    }
}

fn reveal_question(group: &Group<'_>, listed: usize) -> Question {
    Question {
        id: group.name.to_string(),
        instructions: format!(
            "The \"{}\" tools were hidden from the coding agent earlier in this conversation. \
             Does the user's latest request need them now? Showing them again re-reads the whole \
             conversation once, so answer yes only when they are clearly needed. Tools: {}",
            group.name,
            listing(group, listed)
        ),
        kind: QuestionKind::Noul,
    }
}

/// Which of the candidates, given as `(id, description)`, fits, or none.
fn choose_question(
    candidates: &[(String, String)],
    user_request: &str,
    evidence: &str,
) -> Question {
    let mut options: Vec<ChoiceOption> = candidates
        .iter()
        .map(|(id, description)| ChoiceOption {
            value: id.clone(),
            description: description.clone(),
        })
        .collect();

    options.push(ChoiceOption {
        value: chauffeur_core::judge::NONE.into(),
        description: "No hidden tool fits".into(),
    });

    Question {
        id: "recover/choose".into(),
        instructions: format!(
            "Which registered, hidden direct tool fits the user's request and the tool result? Request: {user_request}. Evidence: {evidence}. Choose none if no offered tool fits."
        ),
        kind: QuestionKind::Choice { options },
    }
}

/// Whether the agent clearly needs the chosen tool.
fn confirm_question(tool: &str, description: &str, user_request: &str, evidence: &str) -> Question {
    Question {
        id: format!("recover/{tool}"),
        instructions: format!(
            "Does the agent need the hidden direct tool {tool} ({description}) to complete the user's request? Request: {user_request}. Tool result evidence: {evidence}. Reveal only if clearly needed."
        ),
        kind: QuestionKind::Choice {
            options: [REVEAL_NEEDED, "keep_not_needed", "keep_uncertain"]
                .into_iter()
                .map(|value| ChoiceOption {
                    value: value.into(),
                    description: value.replace('_', " "),
                })
                .collect(),
        },
    }
}
