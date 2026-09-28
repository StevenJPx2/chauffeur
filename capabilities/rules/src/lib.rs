//! Rules: on a tool result or a turn end, every rule whose gate admits the
//! agent's exact facts asks Jev its first step; a rule whose steps all hold
//! delivers its context, highest priority first. One format serves shipped
//! rules (`skills/rules/`) and a project's own (`.chauffeur/rules/`). Run it
//! as `Judging::new(Rules::new(rules))`, with your limits as
//! `Rules::new(rules).with_config(RulesConfig::load(path)?)`.

mod book;
mod history;
mod rule;
mod rulebook;
mod source;

use std::collections::{HashMap, HashSet};
use std::path::Path;

use chauffeur_core::judge::{
    self,
    strategy::{self, Chained},
};
use chauffeur_core::{
    Effect, Judge, Judged, Question, QuestionKind, Signal, SignalKind, Situation, load_layered,
};
use serde::{Deserialize, Deserializer, Serialize};

use book::{Books, Notice, Running};
use history::Histories;
pub use rule::{Gate, History, Rule, SCHEMA_VERSION, Step, Then, Trigger};
pub use rulebook::{
    Args, End, Input, InputSkills, RULEBOOK_SCHEMA_VERSION, Rulebook, load_rulebooks,
};

/// The rulebooks offered in `workspace`: `shipped` ones whose scope covers it,
/// then the project's own. `home` expands `~/` in a scope.
///
/// # Errors
///
/// An unreadable or invalid project rulebook.
pub fn rulebooks_for(shipped: Vec<Rulebook>, workspace: &str) -> Result<Vec<Rulebook>, String> {
    Books::new(shipped, true).available(workspace)
}
pub use source::{load_dir, load_project};

/// The rules that held for a signal, or a rulebook command's answer.
pub struct Verdict {
    rules: Vec<Rule>,
    notice: Option<Notice>,
    /// Running books whose turn-end rules were asked, so each one's
    /// `otherwise` applies when none of its rules holds.
    book_turn: Vec<String>,
}

pub const ID: &str = "rules";
/// The most contexts any config lets one signal deliver.
pub const MAX_DELIVERIES_CAP: usize = 8;
const MAX_FIRED: usize = 4_096;
/// The shipped limits (`skills/config/rules.json`), compiled in.
const SHIPPED: &str = include_str!("../../../skills/config/rules.json");

/// How much the rules deliver.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RulesConfig {
    /// Contexts one signal delivers, at most; from 1 to
    /// [`MAX_DELIVERIES_CAP`].
    #[serde(deserialize_with = "deliveries")]
    max_deliveries: usize,
}

impl RulesConfig {
    /// The shipped limits overlaid by your `rules.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, or `max_deliveries` is outside
    /// `1..=`[`MAX_DELIVERIES_CAP`].
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered(SHIPPED, path)
    }

    /// Contexts one signal delivers, at most.
    #[must_use]
    pub const fn max_deliveries(&self) -> usize {
        self.max_deliveries
    }
}

impl Default for RulesConfig {
    fn default() -> Self {
        serde_json::from_str(SHIPPED).expect("shipped rules limits are valid")
    }
}

fn deliveries<'de, D: Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    let count = usize::deserialize(deserializer)?;

    if !(1..=MAX_DELIVERIES_CAP).contains(&count) {
        return Err(serde::de::Error::custom(format!(
            "max_deliveries {count} is outside 1..={MAX_DELIVERIES_CAP}"
        )));
    }

    Ok(count)
}

/// When a rule last delivered to an agent, and whether `once` is spent.
#[derive(Clone, Copy, Deserialize, Serialize)]
struct Fired {
    last_at: u64,
    spent: bool,
    /// A turn-window rule's `once` renews at the next user message.
    history: History,
}

pub struct Rules {
    shipped: Vec<Rule>,
    config: RulesConfig,
    histories: Histories,
    fired: HashMap<(String, String), Fired>,
    /// The latest project-rule error, logged once until it changes.
    project_error: Option<String>,
    books: Books,
}

#[derive(Deserialize)]
struct Saved {
    histories: Histories,
    fired: Vec<((String, String), Fired)>,
    #[serde(default)]
    books: Vec<(String, Vec<Running>)>,
}

/// [`Saved`], borrowed for writing.
#[derive(Serialize)]
struct SavedRef<'a> {
    histories: &'a Histories,
    fired: Vec<(&'a (String, String), Fired)>,
    books: Vec<(&'a String, &'a Vec<Running>)>,
}

impl Rules {
    /// `shipped` rules apply everywhere; project rules load per workspace.
    /// The shipped limits apply.
    #[must_use]
    pub fn new(shipped: Vec<Rule>) -> Self {
        Self {
            shipped,
            config: RulesConfig::default(),
            histories: Histories::default(),
            fired: HashMap::new(),
            project_error: None,
            books: Books::new(Vec::new(), false),
        }
    }

    /// These rules within `config`'s limits.
    #[must_use]
    pub fn with_config(mut self, config: RulesConfig) -> Self {
        self.config = config;
        self
    }

    /// Rulebooks the user can start per session. `continues` says whether
    /// turn-end rules may resume the agent; without it a book's turn-end
    /// rules stay quiet.
    #[must_use]
    pub fn with_rulebooks(mut self, shelf: Vec<Rulebook>, continues: bool) -> Self {
        self.books = Books::new(shelf, continues);
        self
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

    /// Judge every rule `trigger` admits, each step asked only after the
    /// previous one held.
    fn start(
        &mut self,
        signal: &Signal,
        trigger: Trigger,
        workspace: &str,
        tool: Option<&str>,
    ) -> Judge<Vec<Rule>> {
        let agent = &signal.agent_id;
        let book = self.books.rules(agent, trigger);
        let tools = self.histories.turn_tools(agent);
        let rules: Vec<Rule> = self
            .rules_for(workspace)
            .into_iter()
            .chain(book)
            .filter(|rule| rule.on == trigger)
            .filter(|rule| {
                tool.is_none_or(|tool| rule.when.tools.iter().any(|watched| watched == tool))
            })
            .filter(|rule| self.histories.admits(&rule.when, agent, workspace))
            .filter(|rule| self.may_fire(agent, rule, signal.at))
            .collect();
        let candidates = rules.into_iter().map(|rule| Chained {
            steps: rule
                .steps
                .iter()
                .map(|step| {
                    (
                        question(&rule, step, &signal.kind, &tools),
                        judge::Rule::yes(step.yes_at_or_above, step.minimum_confidence),
                    )
                })
                .collect(),
            key: rule,
        });

        strategy::chain(candidates)
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

impl Judged for Rules {
    type Verdict = Verdict;

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
            books: self.books.save(),
        })
        .ok()
    }

    fn load(&mut self, state: serde_json::Value) {
        let Ok(saved) = serde_json::from_value::<Saved>(state) else {
            return;
        };

        self.histories = Histories::restore(saved.histories);
        self.fired = saved.fired.into_iter().take(MAX_FIRED).collect();
        self.books.load(saved.books);
    }

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<Verdict>> {
        let agent = signal.agent_id.as_str();
        let rules = |judge: Judge<Vec<Rule>>| {
            judge.map(|rules| Verdict {
                rules,
                notice: None,
                book_turn: Vec::new(),
            })
        };

        match &signal.kind {
            SignalKind::UserMessage { .. } => {
                self.histories.user_message(agent);
                self.renew(agent);
                self.books.worked(agent);
                None
            }
            SignalKind::IntegrationEvent { source, kind, .. } => {
                self.histories.hook(agent, format!("{source}:{kind}"));
                None
            }
            SignalKind::ToolResult {
                tool, workspace, ..
            } => {
                self.histories.tool(agent, workspace, tool);
                self.books.worked(agent);
                Some(rules(self.start(
                    signal,
                    Trigger::ToolResult,
                    workspace,
                    Some(tool),
                )))
            }
            SignalKind::TurnEnd { workspace, .. } => {
                let book_turn = self.books.turn_books(agent);

                Some(
                    self.start(signal, Trigger::TurnEnd, workspace, None)
                        .map(move |rules| Verdict {
                            rules,
                            notice: None,
                            book_turn,
                        }),
                )
            }
            SignalKind::Rulebook {
                command,
                rulebook,
                args,
                workspace,
            } => Some(Judge::done(Verdict {
                rules: Vec::new(),
                notice: Some(
                    self.books
                        .command(agent, *command, rulebook, args, workspace),
                ),
                book_turn: Vec::new(),
            })),
            _ => None,
        }
    }

    /// A rulebook command's answer, then the confirmed rules' contexts,
    /// highest priority first, within [`RulesConfig::max_deliveries`]. Among
    /// equal priorities, a rule confirmed in an earlier round, having fewer
    /// steps, comes first. A running rulebook delivers one rule per signal, so
    /// the agent never hears both "done" and "keep going", and at a turn end
    /// where none of its rules holds it delivers its `otherwise`.
    fn act(&mut self, signal: &Signal, verdict: Verdict) -> Vec<Effect> {
        let agent = signal.agent_id.as_str();
        let mut delivered: HashSet<String> = HashSet::new();
        let mut effects: Vec<Effect> = verdict
            .notice
            .into_iter()
            .map(|notice| context(agent, notice))
            .collect();

        for (rule, owner) in self.chosen(agent, verdict.rules) {
            self.record_fired(agent, &rule, signal.at);

            let budget = owner.and_then(|id| {
                let spent = self.books.deliver(agent, &id, &rule);

                delivered.insert(id);
                spent
            });

            effects.push(match budget {
                Some(notice) => context(agent, notice),
                None => Effect::Context {
                    agent_id: agent.to_string(),
                    delivery: rule.then.delivery,
                    label: rule.label().to_string(),
                    skills: rule.then.skill.into_iter().collect(),
                    text: Some(rule.then.text),
                },
            });
        }

        // Not done and not blocked: each running book keeps the agent going.
        for id in verdict
            .book_turn
            .iter()
            .filter(|id| !delivered.contains(*id))
        {
            effects.extend(
                self.books
                    .otherwise(agent, id)
                    .map(|notice| context(agent, notice)),
            );
        }

        effects
    }
}

impl Rules {
    /// The confirmed rules to deliver, highest priority first, at most one
    /// per running book, within the delivery limit; each with its book.
    fn chosen(&self, agent: &str, mut confirmed: Vec<Rule>) -> Vec<(Rule, Option<String>)> {
        let mut books: HashSet<String> = HashSet::new();

        confirmed.sort_by_key(|rule| (std::cmp::Reverse(rule.priority), rule.steps.len()));

        confirmed
            .into_iter()
            .map(|rule| {
                let owner = self.books.owner(agent, &rule);

                (rule, owner)
            })
            .filter(|(_, owner)| owner.as_ref().is_none_or(|id| books.insert(id.clone())))
            .take(self.config.max_deliveries())
            .collect()
    }
}

fn context(agent: &str, notice: Notice) -> Effect {
    Effect::Context {
        agent_id: agent.to_string(),
        delivery: notice.delivery,
        label: notice.label,
        skills: notice.skills,
        text: Some(notice.text),
    }
}

fn question_id(rule: &Rule, step: &Step) -> String {
    format!("{}/{}", rule.id, step.id)
}

/// A step's question with what it is about: the call a tool-result rule
/// judges, or at a turn end the user's latest request, the agent's closing
/// message, and the tools it called since that request.
fn question(rule: &Rule, step: &Step, kind: &SignalKind, tools: &[String]) -> Question {
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
        SignalKind::TurnEnd {
            user_request,
            summary,
            ..
        } => turn_end_question(&step.question, user_request, summary, tools),
        _ => step.question.clone(),
    };

    Question {
        id: question_id(rule, step),
        instructions,
        kind: QuestionKind::Noul,
    }
}

fn turn_end_question(
    question: &str,
    user_request: &str,
    summary: &str,
    tools: &[String],
) -> String {
    let mut text = question.to_string();

    if !user_request.trim().is_empty() {
        text.push_str(&format!(
            " Latest user request: {user_request}. Respect explicit user instructions."
        ));
    }
    if !summary.trim().is_empty() {
        text.push_str(&format!(
            "\nThe agent's closing message this turn: {summary}"
        ));
    }
    if !tools.is_empty() {
        text.push_str(&format!(
            "\nTools it called since that request: {}.",
            tools.join(", ")
        ));
    }

    text
}
