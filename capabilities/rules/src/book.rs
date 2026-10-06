//! Rulebooks running in sessions: which books each agent runs, with what
//! arguments, how much of each budget is spent, and whether the agent worked
//! since a book last continued it. A running book is a copy of its rulebook
//! taken at start, so a project's edit applies from the next start.

use std::collections::HashMap;

use chauffeur_core::{Delivery, RulebookCommand};
use serde::{Deserialize, Serialize};

use crate::rule::{Rule, Trigger};
use crate::rulebook::{End, Rulebook, load_project_rulebooks};
use crate::texts::RulesTexts;

const MAX_SESSIONS: usize = 256;
/// Books one session runs at once, such as a goal inside a ticket.
const MAX_RUNNING: usize = 4;

/// One running book in a session.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Running {
    pub book: Rulebook,
    pub args: String,
    pub paused: bool,
    pub deliveries: u32,
    /// A tool ran or the user wrote since the book last continued the agent,
    /// so a turn-end check is worth asking. Without it a book could keep
    /// resuming an agent that does nothing.
    pub worked: bool,
}

/// What Chauffeur tells the session about a book.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notice {
    pub delivery: Delivery,
    pub label: String,
    pub text: String,
    /// Skills handed over with it, on a start or resume.
    pub skills: Vec<String>,
}

/// The shipped rulebooks and each session's running books.
pub struct Books {
    shipped: Vec<Rulebook>,
    sessions: HashMap<String, Vec<Running>>,
    /// Turn-end rules may resume the agent (`CHAUFFEUR_IDLE_STEERING`).
    continues: bool,
    /// Expands `~/` in a book's scope.
    home: String,
}

impl Books {
    #[must_use]
    pub fn new(shipped: Vec<Rulebook>, continues: bool) -> Self {
        Self {
            shipped,
            sessions: HashMap::new(),
            continues,
            home: std::env::var("HOME").unwrap_or_default(),
        }
    }

    pub fn save(&self) -> Vec<(&String, &Vec<Running>)> {
        self.sessions.iter().collect()
    }

    pub fn load(&mut self, saved: Vec<(String, Vec<Running>)>) {
        self.sessions = saved
            .into_iter()
            .take(MAX_SESSIONS)
            .map(|(agent, mut running)| {
                running.truncate(MAX_RUNNING);
                (agent, running)
            })
            .collect();
    }

    /// The books offered in `workspace`: shipped ones in scope, then the
    /// project's own. A project book reusing a shipped ID is left out.
    ///
    /// # Errors
    ///
    /// An unreadable or invalid project rulebook.
    pub fn available(&self, workspace: &str) -> Result<Vec<Rulebook>, String> {
        let shipped: Vec<Rulebook> = self
            .shipped
            .iter()
            .filter(|book| book.in_scope(workspace, &self.home))
            .cloned()
            .collect();
        let project = load_project_rulebooks(workspace)?
            .into_iter()
            .filter(|book| self.shipped.iter().all(|shipped| shipped.id != book.id));

        Ok(shipped.into_iter().chain(project).collect())
    }

    fn running(&self, agent: &str) -> &[Running] {
        self.sessions.get(agent).map_or(&[], Vec::as_slice)
    }

    fn find(&mut self, agent: &str, id: &str) -> Option<&mut Running> {
        self.sessions
            .get_mut(agent)?
            .iter_mut()
            .find(|running| running.book.id == id)
    }

    fn stop(&mut self, agent: &str, id: &str) {
        if let Some(running) = self.sessions.get_mut(agent) {
            running.retain(|running| running.book.id != id);
        }
    }

    /// Whether `running` asks its turn-end rules now.
    fn checks_turn(&self, running: &Running) -> bool {
        running.worked && self.continues
    }

    /// The running books' rules for `trigger`, with their arguments filled
    /// in. Turn-end rules wait until the agent has worked since the book
    /// last continued it.
    #[must_use]
    pub fn rules(&self, agent: &str, trigger: Trigger) -> Vec<Rule> {
        self.running(agent)
            .iter()
            .filter(|running| !running.paused)
            .filter(|running| trigger != Trigger::TurnEnd || self.checks_turn(running))
            .flat_map(|running| running.book.rules_with(&running.args))
            .filter(|rule| rule.on == trigger)
            .collect()
    }

    /// The books whose turn-end rules [`Books::rules`] asks now.
    #[must_use]
    pub fn turn_books(&self, agent: &str) -> Vec<String> {
        self.running(agent)
            .iter()
            .filter(|running| !running.paused && self.checks_turn(running))
            .filter(|running| {
                running
                    .book
                    .rules
                    .iter()
                    .any(|rule| rule.on == Trigger::TurnEnd)
            })
            .map(|running| running.book.id.clone())
            .collect()
    }

    /// A tool ran or the user wrote.
    pub fn worked(&mut self, agent: &str) {
        for running in self.sessions.get_mut(agent).into_iter().flatten() {
            running.worked = true;
        }
    }

    /// The running book `rule` belongs to, by the longest matching ID.
    #[must_use]
    pub fn owner(&self, agent: &str, rule: &Rule) -> Option<String> {
        self.running(agent)
            .iter()
            .map(|running| &running.book.id)
            .filter(|id| rule.id.starts_with(&format!("{id}-")))
            .max_by_key(|id| id.len())
            .cloned()
    }

    /// Record that `rule` of book `id` is delivering. A spent budget
    /// replaces the delivery with the book's budget notice and stops it.
    pub fn deliver(&mut self, agent: &str, id: &str, rule: &Rule) -> Option<Notice> {
        if let Some(spent) = self.spent(agent, id) {
            return Some(spent);
        }

        let running = self.find(agent, id)?;
        running.deliveries = running.deliveries.saturating_add(1);

        if rule.then.delivery == Delivery::Resume {
            running.worked = false;
        }
        match rule.then.end {
            Some(End::Complete) => self.stop(agent, id),
            Some(End::Pause) => running.paused = true,
            None => {}
        }

        None
    }

    /// Book `id`'s `otherwise`, for a turn end where its turn-end rules were
    /// asked and none holds; within the budget like any delivery.
    pub fn otherwise(&mut self, agent: &str, id: &str) -> Option<Notice> {
        if let Some(spent) = self.spent(agent, id) {
            return Some(spent);
        }

        let running = self.find(agent, id).filter(|running| !running.paused)?;
        let text = Rulebook::say(running.book.otherwise.as_deref()?, &running.args);

        running.deliveries = running.deliveries.saturating_add(1);
        running.worked = false;

        Some(notice(Delivery::Resume, &running.book.name, text))
    }

    /// The budget notice, stopping book `id`, once its deliveries are spent.
    fn spent(&mut self, agent: &str, id: &str) -> Option<Notice> {
        let running = self.find(agent, id)?;

        if running.deliveries < running.book.budget {
            return None;
        }

        let notice = notice(
            Delivery::Resume,
            &running.book.name,
            Rulebook::say(&running.book.on_budget, &running.args),
        );
        self.stop(agent, id);

        Some(notice)
    }

    /// Carry out the user's command in `workspace` and say what happened.
    pub fn command(
        &mut self,
        agent: &str,
        command: RulebookCommand,
        id: &str,
        args: &str,
        workspace: &str,
        texts: &RulesTexts,
    ) -> Notice {
        if command == RulebookCommand::Start {
            return self.start(agent, id, args, workspace, texts);
        }

        let Some(running) = self.find(agent, id).map(|running| running.clone()) else {
            return notice(Delivery::Wait, id, texts.not_running.render(&[("id", id)]));
        };
        let name = running.book.name.clone();

        match command {
            RulebookCommand::Status => {
                let state = if running.paused {
                    &texts.state_paused
                } else {
                    &texts.state_running
                };

                notice(
                    Delivery::Wait,
                    &name,
                    texts.status.render(&[
                        ("name", &name),
                        ("state", &state.render(&[])),
                        ("args", &running.args),
                        ("used", &running.deliveries.to_string()),
                        ("budget", &running.book.budget.to_string()),
                    ]),
                )
            }
            RulebookCommand::Pause => {
                self.set(agent, id, |running| running.paused = true);
                notice(
                    Delivery::Wait,
                    &name,
                    texts.paused.render(&[("name", &name), ("id", id)]),
                )
            }
            RulebookCommand::Resume => {
                self.set(agent, id, |running| {
                    running.paused = false;
                    running.worked = true;
                });
                started(&running.book, &running.args, "")
            }
            RulebookCommand::Clear | RulebookCommand::Start => {
                self.stop(agent, id);
                notice(
                    Delivery::Wait,
                    &name,
                    texts.cleared.render(&[("name", &name)]),
                )
            }
        }
    }

    fn start(
        &mut self,
        agent: &str,
        id: &str,
        args: &str,
        workspace: &str,
        texts: &RulesTexts,
    ) -> Notice {
        let books = match self.available(workspace) {
            Ok(books) => books,
            Err(error) => {
                return notice(
                    Delivery::Wait,
                    id,
                    texts.unreadable.render(&[("error", &error)]),
                );
            }
        };
        let Some(book) = books.iter().find(|book| book.id == id).cloned() else {
            let known: Vec<String> = books.iter().map(|book| format!("/{}", book.id)).collect();

            return notice(
                Delivery::Wait,
                id,
                texts
                    .unknown_book
                    .render(&[("id", id), ("known", &known.join(", "))]),
            );
        };
        let args = match book.accept(args) {
            Ok(args) => args.to_string(),
            Err(error) => return notice(Delivery::Wait, &book.name, error),
        };

        self.stop(agent, id);
        if self.running(agent).len() >= MAX_RUNNING {
            return notice(
                Delivery::Wait,
                &book.name,
                texts
                    .session_full
                    .render(&[("max", &MAX_RUNNING.to_string())]),
            );
        }
        if !self.sessions.contains_key(agent) && self.sessions.len() >= MAX_SESSIONS {
            self.sessions.clear();
        }

        let blocked = if self.continues || book.rules.iter().all(|rule| rule.on != Trigger::TurnEnd)
        {
            String::new()
        } else {
            texts.idle_steering_off.render(&[])
        };
        let answer = started(&book, &args, &blocked);

        self.sessions
            .entry(agent.to_string())
            .or_default()
            .push(Running {
                book,
                args,
                paused: false,
                deliveries: 0,
                worked: true,
            });

        answer
    }

    fn set(&mut self, agent: &str, id: &str, change: impl FnOnce(&mut Running)) {
        if let Some(running) = self.find(agent, id) {
            change(running);
        }
    }
}

/// The start or resume answer: `on_start` with the book's skills, waking the
/// agent.
fn started(book: &Rulebook, args: &str, suffix: &str) -> Notice {
    Notice {
        delivery: Delivery::Resume,
        label: book.name.clone(),
        text: format!("{}{suffix}", Rulebook::say(&book.on_start, args)),
        skills: book.skills_for(args),
    }
}

fn notice(delivery: Delivery, label: &str, text: String) -> Notice {
    Notice {
        delivery,
        label: label.to_string(),
        text,
        skills: Vec::new(),
    }
}
