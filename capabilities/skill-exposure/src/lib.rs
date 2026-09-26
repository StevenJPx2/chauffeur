//! Skill exposure: System One picks the one skill, or none, that best helps.
//! It asks on each user message, when the agent asks for help itself, and
//! again after tool results and at turn end in case the agent lost track,
//! such as driving a browser where a dedicated skill exists.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use chauffeur_core::{
    Answer, AnswerValue, Capability, CatalogEntry, ChoiceOption, Delivery, Effect, PipeStep, Plan,
    Question, QuestionKind, Signal, SignalKind, Situation,
};

pub const ID: &str = "skill-exposure";
/// The option that attaches nothing.
pub const NONE: &str = "none";
/// Attach only when the choice is at least this confident.
pub const MIN_CONFIDENCE: f32 = 0.4;
/// Mid-turn hand-overs need stronger evidence than prompt admission.
pub const DRIFT_MIN_CONFIDENCE: f32 = 0.7;
/// Skill bytes one signal may attach, about 16k tokens: any two skills a
/// prompt needs fit, and no pick can flood the context. Jev's confidence
/// decides whether a skill helps; this only bounds what attaching costs.
pub const MAX_SIGNAL_BYTES: u64 = 65_536;
/// Skills offered per question; the catalog is truncated in host order.
pub const MAX_OPTIONS: usize = 64;
/// Tool results between drift checks are skipped for this long, per agent.
pub const DRIFT_COOLDOWN_SECS: u64 = 60;
const MAX_AGENTS: usize = 256;
const PICK: &str = "pick";
/// A prompt's second pick, asked once the first attached a skill.
const ALSO: &str = "also";
/// Question ID prefix for a skill named after the session's project…
const PROJECT: &str = "project:";
/// …and for one the request names, such as `slack` in a Slack link.
const NAMED: &str = "named:";
const MAX_FOCUSED_SKILLS: usize = 3;
/// A project skill applies in its project unless Jev confidently says the
/// request is about something else: P(involves) at most this withholds it.
pub const PROJECT_WITHHOLD_AT_OR_BELOW: f32 = 0.3;
/// A named skill attaches when P(needed) is at least this.
pub const NAMED_AT_OR_ABOVE: f32 = 0.7;
const DRIFT: &str = "drift";
const ASK: &str = "ask";
const BROWSER_HARNESS: &str = "browser-harness";

/// What one agent can still be offered, from its latest user message.
#[derive(Clone, Default, Deserialize, Serialize)]
struct Agent {
    skills: Vec<CatalogEntry>,
    attached: HashSet<String>,
    last_drift_at: Option<u64>,
    #[serde(default)]
    tools_seen: u32,
    #[serde(default)]
    drift_seen: u32,
    #[serde(default)]
    last_tool: Option<String>,
    #[serde(default)]
    browser_tool: bool,
}

impl Agent {
    fn offerable(&self) -> Vec<&CatalogEntry> {
        let mut seen = HashSet::new();

        self.skills
            .iter()
            .filter(|skill| {
                skill.id != NONE
                    && !self.attached.contains(&skill.id)
                    && seen.insert(skill.id.as_str())
            })
            .take(MAX_OPTIONS)
            .collect()
    }

    fn drift_due(&self, at: u64) -> bool {
        self.last_drift_at
            .is_none_or(|last| at.saturating_sub(last) >= DRIFT_COOLDOWN_SECS)
    }

    /// Remember the latest tool action and whether it drove a browser, so a
    /// browser hand-over is offered only for actual browser use.
    fn record_tool(&mut self, tool: &str, ok: bool, input: &str) {
        self.tools_seen = self.tools_seen.saturating_add(1);
        self.last_tool = Some(format!("tool: {tool}; succeeded: {ok}; input: {input}"));
        self.browser_tool = tool.starts_with("browser")
            || (matches!(tool, "shell" | "execute")
                && (input.contains(BROWSER_HARNESS) || input.contains("tools.browser.")));
    }

    /// Whether `skill` may be offered by the question `id`.
    fn admits(&self, id: &str, skill: &str) -> bool {
        id != DRIFT || skill != BROWSER_HARNESS || self.browser_tool
    }
}

#[derive(Default)]
pub struct SkillExposure {
    agents: HashMap<String, Agent>,
    /// A prompt's first pick, held while the second is judged.
    held: Vec<Effect>,
    /// Skill bytes attached for the current signal.
    spent: u64,
}

impl SkillExposure {
    fn agent(&mut self, agent_id: &str) -> &mut Agent {
        if !self.agents.contains_key(agent_id) && self.agents.len() >= MAX_AGENTS {
            self.agents.clear();
        }

        self.agents.entry(agent_id.to_string()).or_default()
    }

    /// Attach an offerable skill within the signal's byte budget.
    fn attach(&mut self, signal: &Signal, skill: &str) -> Option<Effect> {
        let remaining = MAX_SIGNAL_BYTES.saturating_sub(self.spent);
        let agent = self.agent(&signal.agent_id);
        let bytes = agent
            .offerable()
            .iter()
            .find(|offered| offered.id == skill)
            .map(|offered| u64::from(offered.bytes))
            .filter(|bytes| *bytes <= remaining)?;

        agent.attached.insert(skill.to_string());
        self.spent = self.spent.saturating_add(bytes);

        Some(context(signal, skill))
    }

    /// The focused skills whose answers admit them: a project skill unless
    /// confidently unrelated, a named skill when confidently needed. A failed
    /// judgment attaches neither.
    fn focused_picks(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        let admitted: Vec<String> = answers
            .unwrap_or_default()
            .iter()
            .filter_map(|answer| {
                let AnswerValue::Noul(p) = answer.value else {
                    return None;
                };
                let confident = answer.effective_confidence() >= MIN_CONFIDENCE;

                if let Some(skill) = answer.id.strip_prefix(PROJECT) {
                    return (!(p <= PROJECT_WITHHOLD_AT_OR_BELOW && confident))
                        .then(|| skill.to_string());
                }

                answer
                    .id
                    .strip_prefix(NAMED)
                    .filter(|_| p >= NAMED_AT_OR_ABOVE && confident)
                    .map(str::to_string)
            })
            .collect();

        admitted
            .iter()
            .filter_map(|skill| self.attach(signal, skill))
            .collect()
    }

    /// After a prompt's first pick: which other skill it also needs.
    fn also_question(&self, signal: &Signal, picked: &[Effect]) -> Option<Question> {
        let Some(Effect::Context { skills, .. }) = picked.first() else {
            return None;
        };
        let first = skills.first()?;
        let agent = self.agents.get(&signal.agent_id)?;
        let remaining = MAX_SIGNAL_BYTES.saturating_sub(self.spent);
        let offerable: Vec<_> = agent
            .offerable()
            .into_iter()
            .filter(|skill| u64::from(skill.bytes) <= remaining)
            .collect();

        if offerable.is_empty() {
            return None;
        }

        let instructions = format!(
            "The {first} skill is already attached for the user's latest request. Which one \
             other offered skill does that request also clearly need? Choose none unless one \
             clearly does.{}",
            where_session_works(signal)
        );

        Some(question(ALSO, &instructions, &offerable))
    }
}

/// Why a skill gets its own question, from exact facts about the prompt.
#[derive(Clone, Copy)]
enum Focus {
    /// Named after a directory the session works in, such as `hpdp-overlay`
    /// for `…/hpdp-overlay/ADEPT-45130`.
    Project,
    /// Its leading word is a word of the request, such as `slack` in
    /// `adeptmind.slack.com` for `slack-cli`.
    Named,
}

/// The skills a prompt's facts point at, each with why. Jev still judges
/// each one.
fn focused_skills<'a>(
    signal: &Signal,
    offerable: &[&'a CatalogEntry],
) -> Vec<(&'a CatalogEntry, Focus)> {
    let SignalKind::UserMessage {
        text, workspace, ..
    } = &signal.kind
    else {
        return Vec::new();
    };
    let words = |value: &str| -> Vec<String> {
        value
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
            .filter(|word| !word.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    };
    let (directories, said) = (words(workspace), words(&text.replace('-', " ")));

    offerable
        .iter()
        .copied()
        .filter_map(|skill| {
            let id = skill.id.to_ascii_lowercase();
            let lead = id.split('-').next().unwrap_or_default();

            if directories.contains(&id) {
                Some((skill, Focus::Project))
            } else {
                (lead.len() >= 3 && said.iter().any(|word| word == lead))
                    .then_some((skill, Focus::Named))
            }
        })
        .take(MAX_FOCUSED_SKILLS)
        .collect()
}

fn focused_question(signal: &Signal, skill: &CatalogEntry, focus: Focus) -> Question {
    let (prefix, instructions) = match focus {
        Focus::Project => (
            PROJECT,
            format!(
                "The coding session works inside the {id} project ({}). The {id} skill: {}. Does \
                 the user's latest request involve work in this project that the skill covers? \
                 Answer no only when the request is clearly about something else.",
                where_session_works(signal).trim(),
                skill.description,
                id = skill.id,
            ),
        ),
        Focus::Named => (
            NAMED,
            format!(
                "The user's latest request mentions {lead}. The {id} skill: {}. Does the agent \
                 need this skill to act on the request?",
                skill.description,
                id = skill.id,
                lead = skill.id.split('-').next().unwrap_or_default(),
            ),
        ),
    };

    Question {
        id: format!("{prefix}{}", skill.id),
        instructions,
        kind: QuestionKind::Noul,
    }
}

/// A question's instructions with what it is about.
fn asked(agent: &Agent, signal: &Signal, id: &str, instructions: &str) -> String {
    match &signal.kind {
        _ if id == DRIFT => format!(
            "{instructions} Latest tool action: {}",
            agent.last_tool.as_deref().unwrap_or("unknown")
        ),
        SignalKind::AgentRequest {
            need, user_request, ..
        } => format!(
            "{instructions} The agent asked for: \"{need}\". The user's latest request: \
             {user_request}."
        ),
        _ => format!("{instructions}{}", where_session_works(signal)),
    }
}

/// Where a prompt's session works, as a sentence; empty when unknown.
fn where_session_works(signal: &Signal) -> String {
    match &signal.kind {
        SignalKind::UserMessage { workspace, .. } if !workspace.is_empty() => {
            format!(" The session works in {workspace}.")
        }
        _ => String::new(),
    }
}

impl Capability for SkillExposure {
    fn id(&self) -> &str {
        ID
    }

    fn save(&self) -> Option<serde_json::Value> {
        serde_json::to_value(&self.agents).ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        if let Ok(agents) = serde_json::from_value::<HashMap<String, Agent>>(state) {
            self.agents = agents.into_iter().take(MAX_AGENTS).collect();
        }
    }

    fn plan(&mut self, _: &Situation, signal: &Signal) -> Plan {
        self.held.clear();
        self.spent = 0;

        let (id, instructions) = match &signal.kind {
            SignalKind::UserMessage { skills, .. } => {
                // The host lists only skills not yet attached in this context.
                *self.agent(&signal.agent_id) = Agent {
                    skills: skills.clone(),
                    ..Agent::default()
                };

                (PICK, PICK_INSTRUCTIONS)
            }
            SignalKind::ToolResult { .. } | SignalKind::TurnEnd { .. } => {
                let Some(agent) = self.agents.get_mut(&signal.agent_id) else {
                    return Plan::Skip;
                };
                let turn_end = matches!(signal.kind, SignalKind::TurnEnd { .. });

                if let SignalKind::ToolResult {
                    tool, ok, input, ..
                } = &signal.kind
                {
                    agent.record_tool(tool, *ok, input);
                }

                if agent.tools_seen == agent.drift_seen
                    || (!turn_end && !agent.drift_due(signal.at))
                {
                    return Plan::Skip;
                }

                agent.last_drift_at = Some(signal.at);
                agent.drift_seen = agent.tools_seen;

                (DRIFT, DRIFT_INSTRUCTIONS)
            }
            SignalKind::AgentRequest { .. } => (ASK, ASK_INSTRUCTIONS),
            _ => return Plan::Skip,
        };
        let Some(agent) = self.agents.get(&signal.agent_id) else {
            return Plan::Skip;
        };
        let offerable: Vec<_> = agent
            .offerable()
            .into_iter()
            .filter(|skill| agent.admits(id, &skill.id))
            .collect();
        let focused = focused_skills(signal, &offerable);
        let rest: Vec<_> = offerable
            .into_iter()
            .filter(|skill| !focused.iter().any(|(named, _)| named.id == skill.id))
            .collect();
        let mut questions: Vec<Question> = focused
            .iter()
            .map(|(skill, focus)| focused_question(signal, skill, *focus))
            .collect();

        if !rest.is_empty() {
            questions.insert(
                0,
                question(id, &asked(agent, signal, id, instructions), &rest),
            );
        }

        if questions.is_empty() {
            return Plan::Skip;
        }

        Plan::Ask(questions)
    }

    /// A prompt's first pick may bring a second: a request can need two
    /// skills, such as a tool's skill and the project's own.
    fn advance(&mut self, signal: &Signal, answers: Option<&[Answer]>, round: usize) -> PipeStep {
        let mut picked = self.decide(signal, answers);

        picked.extend(self.focused_picks(signal, answers));

        if round > 1 || !matches!(signal.kind, SignalKind::UserMessage { .. }) {
            let mut effects = std::mem::take(&mut self.held);

            effects.extend(picked);

            return PipeStep::Done(effects);
        }

        // Two skills from the first round are already the most a prompt gets.
        let also = (picked.len() == 1)
            .then(|| self.also_question(signal, &picked))
            .flatten();

        match also {
            Some(question) => {
                self.held = picked;

                PipeStep::Next(vec![question])
            }
            None => PipeStep::Done(picked),
        }
    }

    fn decide(&mut self, signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
        // Enhancements fail open: a failed judgment attaches nothing.
        let Some(answer) = answers.and_then(|answers| {
            answers
                .iter()
                .find(|answer| [PICK, ALSO, DRIFT, ASK].contains(&answer.id.as_str()))
        }) else {
            return Vec::new();
        };
        let AnswerValue::Choice(choice) = &answer.value else {
            return Vec::new();
        };

        let minimum = if answer.id == DRIFT {
            DRIFT_MIN_CONFIDENCE
        } else {
            MIN_CONFIDENCE
        };

        if choice == NONE || answer.effective_confidence() < minimum {
            return Vec::new();
        }

        if !self.agent(&signal.agent_id).admits(&answer.id, choice) {
            return Vec::new();
        }

        let choice = choice.clone();

        self.attach(signal, &choice).into_iter().collect()
    }
}

/// A picked skill joins the user's message; a skill the agent asked for
/// answers it in the running turn; a drift hand-over reaches the running
/// turn, or waits for the next one once the turn has ended.
fn context(signal: &Signal, skill: &str) -> Effect {
    let (delivery, text) = match signal.kind {
        SignalKind::UserMessage { .. } => (Delivery::Prompt, None),
        SignalKind::AgentRequest { .. } => (Delivery::Steer, None),
        SignalKind::TurnEnd { .. } => (Delivery::Wait, Some(drift_text(skill))),
        _ => (Delivery::Steer, Some(drift_text(skill))),
    };

    Effect::Context {
        agent_id: signal.agent_id.clone(),
        delivery,
        label: format!("skill {skill}"),
        skills: vec![skill.to_string()],
        text,
    }
}

fn drift_text(skill: &str) -> String {
    format!("Chauffeur: the {skill} skill fits this work better than the current approach.")
}

const PICK_INSTRUCTIONS: &str = "Which one skill would most help the coding agent act on the \
    user's latest request? Choose none unless a skill clearly helps.";

const ASK_INSTRUCTIONS: &str = "Which one offered skill serves what the coding agent asked \
    for? Choose none unless a skill clearly does.";

const DRIFT_INSTRUCTIONS: &str = "Which offered skill, if any, directly improves the exact \
    action in the agent's recent tool result? Choose none if the approach already works. \
    A skill covering the same topic is not evidence of misuse; successful gh use for GitHub \
    does not call for a browser or browser-harness hand-over.";

fn question(id: &str, instructions: &str, skills: &[&CatalogEntry]) -> Question {
    let mut options: Vec<ChoiceOption> = skills
        .iter()
        .map(|skill| ChoiceOption {
            value: skill.id.clone(),
            description: skill.description.clone(),
        })
        .collect();

    options.push(ChoiceOption {
        value: NONE.into(),
        description: "No skill clearly helps.".into(),
    });

    Question {
        id: id.into(),
        instructions: instructions.into(),
        kind: QuestionKind::Choice { options },
    }
}
