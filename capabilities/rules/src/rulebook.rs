//! The rulebook format: a named set of rules the user starts in a session,
//! with arguments, and stops again. Its rules use the rule format, with
//! `{args}` in their questions and texts and `then.end` to finish the book.

use std::path::Path;

use chauffeur_core::{Delivery, read_json_files};
use serde::{Deserialize, Serialize};

use crate::rule::{Rule, Then, Trigger};

pub const RULEBOOK_SCHEMA_VERSION: u8 = 1;
const MAX_RULEBOOKS: usize = 32;
const MAX_FILE_BYTES: u64 = 32_768;
/// Where a rulebook's arguments go in its questions and texts.
pub const ARGS: &str = "{args}";
const MAX_RULES: usize = 8;
const MAX_ARGS_BYTES: usize = chauffeur_core::MAX_PROMPT_BYTES;
const MAX_BUDGET: u32 = 200;
const MAX_TEXT_BYTES: usize = 1_024;
const MAX_DESCRIPTION_BYTES: usize = 256;

/// A named set of rules, dormant until the user starts it in a session.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Rulebook {
    pub schema_version: u8,
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub args: Args,
    /// Deliveries per start, at most; the next one is `on_budget` instead.
    pub budget: u32,
    pub rules: Vec<Rule>,
    /// Delivered, waking the agent, when the book starts.
    pub on_start: String,
    /// Delivered, waking the agent, when the budget runs out.
    pub on_budget: String,
    /// Delivered, waking the agent, at a turn end where the book's turn-end
    /// rules were asked and none holds: the book is not finished, so the
    /// agent keeps going. Without it, such a turn delivers nothing.
    #[serde(default)]
    pub otherwise: Option<String>,
}

/// Whether a rulebook takes arguments.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    /// Starting without arguments is refused.
    #[serde(default)]
    pub required: bool,
}

/// How a rulebook rule ends its book once it delivers.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    /// The book's purpose is met: stop it.
    Complete,
    /// Only the user can unblock it: pause it until `/<book> resume`.
    Pause,
}

impl Rulebook {
    /// # Errors
    ///
    /// Invalid JSON or a rulebook outside the format's bounds.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let book: Self = serde_json::from_slice(bytes).map_err(|error| error.to_string())?;

        book.validate()?;

        Ok(book)
    }

    /// Its rules with `args` in place of every `{args}`.
    #[must_use]
    pub fn rules_with(&self, args: &str) -> Vec<Rule> {
        self.rules
            .iter()
            .cloned()
            .map(|mut rule| {
                for step in &mut rule.steps {
                    step.question = step.question.replace(ARGS, args);
                }
                rule.then.text = rule.then.text.replace(ARGS, args);

                rule
            })
            .collect()
    }

    /// `text` (`on_start` or `on_budget`) with `args` in place.
    #[must_use]
    pub fn say(text: &str, args: &str) -> String {
        text.replace(ARGS, args)
    }

    /// Arguments this book accepts: trimmed, bounded, and present when
    /// required.
    ///
    /// # Errors
    ///
    /// Missing required arguments, or arguments over the bound.
    pub fn accept<'a>(&self, args: &'a str) -> Result<&'a str, String> {
        let args = args.trim();

        if self.args.required && args.is_empty() {
            return Err(format!(
                "{} needs arguments: /{} <what>",
                self.name, self.id
            ));
        }
        if args.len() > MAX_ARGS_BYTES {
            return Err(format!(
                "{} arguments exceed {MAX_ARGS_BYTES} bytes",
                self.name
            ));
        }

        Ok(args)
    }

    fn validate(&self) -> Result<(), String> {
        if self.schema_version != RULEBOOK_SCHEMA_VERSION {
            return Err(format!(
                "unsupported schema_version {} (rulebooks use {RULEBOOK_SCHEMA_VERSION})",
                self.schema_version
            ));
        }
        if !crate::rule::identifier(&self.id) {
            return Err(format!(
                "rulebook id {:?} must be 1-64 of [a-z0-9_-]",
                self.id
            ));
        }

        let within = |text: &str, max: usize| !text.trim().is_empty() && text.len() <= max;

        if !within(&self.name, 128) || !within(&self.description, MAX_DESCRIPTION_BYTES) {
            return Err(format!(
                "{}: name or description is empty or too long",
                self.id
            ));
        }
        let texts = [
            Some(&self.on_start),
            Some(&self.on_budget),
            self.otherwise.as_ref(),
        ];

        if texts
            .into_iter()
            .flatten()
            .any(|text| !within(text, MAX_TEXT_BYTES))
        {
            return Err(format!(
                "{}: on_start, on_budget, and otherwise must be 1-{MAX_TEXT_BYTES} bytes",
                self.id
            ));
        }
        if !(1..=MAX_BUDGET).contains(&self.budget) {
            return Err(format!("{}: budget must be 1-{MAX_BUDGET}", self.id));
        }
        if !(1..=MAX_RULES).contains(&self.rules.len()) {
            return Err(format!("{}: rules must list 1-{MAX_RULES}", self.id));
        }

        self.rules
            .iter()
            .try_for_each(|rule| self.validate_rule(rule))
    }

    fn validate_rule(&self, rule: &Rule) -> Result<(), String> {
        rule.validate_as_part()?;

        if !rule.id.starts_with(&format!("{}-", self.id)) {
            return Err(format!(
                "{}: rule {} must start with \"{}-\"",
                self.id, rule.id, self.id
            ));
        }
        if rule.then.end.is_some() && !fits_end(rule) {
            return Err(format!(
                "{}: rule {} ends the book but does not end a turn",
                self.id, rule.id
            ));
        }

        Ok(())
    }
}

/// A book ends at a turn end, where the agent is idle.
fn fits_end(rule: &Rule) -> bool {
    matches!(
        (rule.on, &rule.then),
        (
            Trigger::TurnEnd,
            Then {
                delivery: Delivery::Resume | Delivery::Wait,
                ..
            }
        )
    )
}

/// Every rulebook in `directory`; a missing directory has none. IDs must be
/// unique.
///
/// # Errors
///
/// An unreadable file, an invalid rulebook, or a repeated ID.
pub fn load_rulebooks(directory: &Path) -> Result<Vec<Rulebook>, String> {
    let mut books: Vec<Rulebook> = Vec::new();

    for (path, bytes) in read_json_files(directory, MAX_RULEBOOKS, MAX_FILE_BYTES)? {
        let book =
            Rulebook::from_json(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;

        if books.iter().any(|known| known.id == book.id) {
            return Err(format!(
                "{}: rulebook id {} is repeated",
                path.display(),
                book.id
            ));
        }

        books.push(book);
    }

    Ok(books)
}
