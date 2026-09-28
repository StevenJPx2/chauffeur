//! Rulebooks running in sessions: which book each agent runs, with what
//! arguments, how much of its budget is spent, and whether the agent worked
//! since the book last continued it.

use std::collections::HashMap;

use chauffeur_core::{Delivery, RulebookCommand};
use serde::{Deserialize, Serialize};

use crate::rule::{Rule, Trigger};
use crate::rulebook::{End, Rulebook};

const MAX_SESSIONS: usize = 256;

/// One session's book.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Running {
    pub book: String,
    pub args: String,
    pub paused: bool,
    pub deliveries: u32,
    /// A tool ran or the user wrote since the book last continued the agent,
    /// so a turn-end check is worth asking. Without it a book could keep
    /// resuming an agent that does nothing.
    pub worked: bool,
}

/// What Chauffeur tells the session about its book.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notice {
    pub delivery: Delivery,
    pub label: String,
    pub text: String,
}

/// The shelf of rulebooks and each session's running book.
pub struct Books {
    shelf: Vec<Rulebook>,
    sessions: HashMap<String, Running>,
    /// Turn-end rules may resume the agent (`CHAUFFEUR_IDLE_STEERING`).
    continues: bool,
}

impl Books {
    #[must_use]
    pub fn new(shelf: Vec<Rulebook>, continues: bool) -> Self {
        Self {
            shelf,
            sessions: HashMap::new(),
            continues,
        }
    }

    pub fn save(&self) -> Vec<(&String, &Running)> {
        self.sessions.iter().collect()
    }

    pub fn load(&mut self, saved: Vec<(String, Running)>) {
        self.sessions = saved
            .into_iter()
            .filter(|(_, running)| self.book(&running.book).is_some())
            .take(MAX_SESSIONS)
            .collect();
    }

    fn book(&self, id: &str) -> Option<&Rulebook> {
        self.shelf.iter().find(|book| book.id == id)
    }

    /// The running book's rules for `trigger`, with its arguments in place.
    /// Turn-end rules wait until the agent has worked since the last
    /// continuation.
    #[must_use]
    pub fn rules(&self, agent: &str, trigger: Trigger) -> Vec<Rule> {
        let Some(running) = self.sessions.get(agent).filter(|running| !running.paused) else {
            return Vec::new();
        };
        let Some(book) = self.book(&running.book) else {
            return Vec::new();
        };

        if trigger == Trigger::TurnEnd && !(running.worked && self.continues) {
            return Vec::new();
        }

        book.rules_with(&running.args)
            .into_iter()
            .filter(|rule| rule.on == trigger)
            .collect()
    }

    /// A tool ran or the user wrote.
    pub fn worked(&mut self, agent: &str) {
        if let Some(running) = self.sessions.get_mut(agent) {
            running.worked = true;
        }
    }

    /// Whether `rule` belongs to `agent`'s running book.
    #[must_use]
    pub fn owns(&self, agent: &str, rule: &Rule) -> bool {
        self.sessions
            .get(agent)
            .is_some_and(|running| rule.id.starts_with(&format!("{}-", running.book)))
    }

    /// Record that `rule` of the running book is delivering. A spent budget
    /// replaces the delivery with the book's budget notice and stops it.
    pub fn deliver(&mut self, agent: &str, rule: &Rule) -> Option<Notice> {
        if let Some(spent) = self.spent(agent) {
            return Some(spent);
        }

        let running = self.sessions.get_mut(agent)?;
        running.deliveries = running.deliveries.saturating_add(1);

        if rule.then.delivery == Delivery::Resume {
            running.worked = false;
        }
        match rule.then.end {
            Some(End::Complete) => {
                self.sessions.remove(agent);
            }
            Some(End::Pause) => running.paused = true,
            None => {}
        }

        None
    }

    /// The running book's `otherwise`, for a turn end where its turn-end
    /// rules were asked and none holds; within the budget like any delivery.
    pub fn otherwise(&mut self, agent: &str) -> Option<Notice> {
        if let Some(spent) = self.spent(agent) {
            return Some(spent);
        }

        let running = self
            .sessions
            .get_mut(agent)
            .filter(|running| !running.paused)?;
        let book = self.shelf.iter().find(|book| book.id == running.book)?;
        let text = Rulebook::say(book.otherwise.as_deref()?, &running.args);

        running.deliveries = running.deliveries.saturating_add(1);
        running.worked = false;

        Some(notice(Delivery::Resume, &book.name, text))
    }

    /// The budget notice, stopping the book, once its deliveries are spent.
    fn spent(&mut self, agent: &str) -> Option<Notice> {
        let running = self.sessions.get(agent)?;
        let book = self.book(&running.book)?;

        if running.deliveries < book.budget {
            return None;
        }

        let notice = notice(
            Delivery::Resume,
            &book.name,
            Rulebook::say(&book.on_budget, &running.args),
        );
        self.sessions.remove(agent);

        Some(notice)
    }

    /// Carry out the user's command and say what happened.
    pub fn command(
        &mut self,
        agent: &str,
        command: RulebookCommand,
        id: &str,
        args: &str,
    ) -> Notice {
        let Some(book) = self.book(id).cloned() else {
            let known: Vec<&str> = self.shelf.iter().map(|book| book.id.as_str()).collect();

            return notice(
                Delivery::Wait,
                "Rulebooks",
                format!("No rulebook named {id}. Rulebooks: {}.", known.join(", ")),
            );
        };
        let running = self
            .sessions
            .get(agent)
            .filter(|running| running.book == id)
            .cloned();

        match (command, running) {
            (RulebookCommand::Start, _) => self.start(agent, &book, args),
            (RulebookCommand::Status, None)
            | (RulebookCommand::Pause | RulebookCommand::Resume | RulebookCommand::Clear, None) => {
                notice(
                    Delivery::Wait,
                    &book.name,
                    format!(
                        "{} is not running. Start it with /{} <what>.",
                        book.name, book.id
                    ),
                )
            }
            (RulebookCommand::Status, Some(running)) => {
                let state = if running.paused { "paused" } else { "running" };

                notice(
                    Delivery::Wait,
                    &book.name,
                    format!(
                        "{} is {state}: {}\n{} of {} continuations used.",
                        book.name, running.args, running.deliveries, book.budget
                    ),
                )
            }
            (RulebookCommand::Pause, Some(_)) => {
                self.set(agent, |running| running.paused = true);
                notice(
                    Delivery::Wait,
                    &book.name,
                    format!("{} paused. Resume it with /{} resume.", book.name, book.id),
                )
            }
            (RulebookCommand::Resume, Some(running)) => {
                self.set(agent, |running| {
                    running.paused = false;
                    running.worked = true;
                });
                notice(
                    Delivery::Resume,
                    &book.name,
                    Rulebook::say(&book.on_start, &running.args),
                )
            }
            (RulebookCommand::Clear, Some(_)) => {
                self.sessions.remove(agent);
                notice(
                    Delivery::Wait,
                    &book.name,
                    format!("{} cleared.", book.name),
                )
            }
        }
    }

    fn start(&mut self, agent: &str, book: &Rulebook, args: &str) -> Notice {
        let args = match book.accept(args) {
            Ok(args) => args.to_string(),
            Err(error) => return notice(Delivery::Wait, &book.name, error),
        };
        let replaced = self
            .sessions
            .get(agent)
            .filter(|running| running.book != book.id)
            .map(|running| format!("Stopped rulebook {}. ", running.book))
            .unwrap_or_default();

        if !self.sessions.contains_key(agent) && self.sessions.len() >= MAX_SESSIONS {
            self.sessions.clear();
        }

        self.sessions.insert(
            agent.to_string(),
            Running {
                book: book.id.clone(),
                args: args.clone(),
                paused: false,
                deliveries: 0,
                worked: true,
            },
        );

        let blocked = if self.continues || book.rules.iter().all(|rule| rule.on != Trigger::TurnEnd)
        {
            ""
        } else {
            "\n(Chauffeur's idle steering is off, so this rulebook cannot continue the agent between turns.)"
        };

        notice(
            Delivery::Resume,
            &book.name,
            format!(
                "{replaced}{}{blocked}",
                Rulebook::say(&book.on_start, &args)
            ),
        )
    }

    fn set(&mut self, agent: &str, change: impl FnOnce(&mut Running)) {
        if let Some(running) = self.sessions.get_mut(agent) {
            change(running);
        }
    }
}

fn notice(delivery: Delivery, label: &str, text: String) -> Notice {
    Notice {
        delivery,
        label: label.to_string(),
        text,
    }
}
