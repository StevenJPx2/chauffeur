//! The rulebook format: a named set of rules the user starts in a session,
//! with arguments, and stops again. Its rules use the rule format, with
//! `{args}` and `{input}` in their questions and texts and `then.end` to
//! finish the book. A shipped book may be scoped to folders; a project's own
//! books live in its `.chauffeur/rulebooks/`.

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

/// Where a description of the arguments goes: "Jira ticket ADEPT-1",
/// "Slack thread <url>", or the text itself.
pub const INPUT: &str = "{input}";

/// Where a turn end's open todos go in a delivered text, one per line, or
/// `none`. Any rule's text may use it; only a turn end carries todos.
pub const TODOS: &str = "{todos}";
const MAX_SCOPES: usize = 8;
const MAX_SKILLS: usize = 4;

/// A named set of rules, dormant until the user starts it in a session.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rulebook {
    pub schema_version: u8,
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub args: Args,
    /// Folders the book is offered in, as absolute or `~/` paths, each
    /// ending in `/**` for the folder and everything under it. Empty offers
    /// it everywhere. A project's own rulebooks are offered in the project.
    #[serde(default)]
    pub scope: Vec<String>,
    /// Skills handed over when the book starts or resumes.
    #[serde(default)]
    pub skills: Vec<String>,
    /// A skill for each kind of input, such as `jira-cli` for a Jira key.
    #[serde(default)]
    pub input_skills: InputSkills,
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

/// The skill handed over for each kind of input.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputSkills {
    #[serde(default)]
    pub jira: Option<String>,
    #[serde(default)]
    pub slack: Option<String>,
}

/// What the user started a book with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Input {
    /// A Jira issue key alone, such as `ADEPT-12345`.
    Jira,
    /// A Slack message or thread link.
    Slack,
    /// Anything else: a request in the user's words.
    Text,
}

impl Input {
    #[must_use]
    pub fn of(args: &str) -> Self {
        let args = args.trim();

        if is_jira_key(args) {
            Self::Jira
        } else if args.starts_with("https://")
            && args.contains(".slack.com/archives/")
            && !args.contains(char::is_whitespace)
        {
            Self::Slack
        } else {
            Self::Text
        }
    }
}

/// `PROJ-123`: an uppercase project key, a hyphen, and digits.
fn is_jira_key(value: &str) -> bool {
    let Some((project, number)) = value.split_once('-') else {
        return false;
    };

    project.len() >= 2
        && project.starts_with(|c: char| c.is_ascii_uppercase())
        && project
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// `text` with the arguments in place of `{args}` and their description in
/// place of `{input}`.
#[must_use]
pub fn fill(text: &str, args: &str) -> String {
    let input = match Input::of(args) {
        Input::Jira => format!("Jira ticket {args}"),
        Input::Slack => format!("Slack thread {args}"),
        Input::Text => args.to_string(),
    };

    text.replace(ARGS, args).replace(INPUT, &input)
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

    /// Its rules with the arguments filled in; see [`fill`].
    #[must_use]
    pub fn rules_with(&self, args: &str) -> Vec<Rule> {
        self.rules
            .iter()
            .cloned()
            .map(|mut rule| {
                for step in &mut rule.steps {
                    step.question = fill(&step.question, args);
                }
                rule.then.text = fill(&rule.then.text, args);

                rule
            })
            .collect()
    }

    /// `text` (`on_start`, `on_budget`, or `otherwise`) with the arguments
    /// filled in; see [`fill`].
    #[must_use]
    pub fn say(text: &str, args: &str) -> String {
        fill(text, args)
    }

    /// The skills handed over on start: the book's own, then its skill for
    /// this kind of input.
    #[must_use]
    pub fn skills_for(&self, args: &str) -> Vec<String> {
        let input = match Input::of(args) {
            Input::Jira => self.input_skills.jira.clone(),
            Input::Slack => self.input_skills.slack.clone(),
            Input::Text => None,
        };

        self.skills.iter().cloned().chain(input).collect()
    }

    /// Whether the book is offered in `workspace`. `home` expands `~/`.
    #[must_use]
    pub fn in_scope(&self, workspace: &str, home: &str) -> bool {
        self.scope.is_empty()
            || self.scope.iter().any(|pattern| {
                let folder = pattern.trim_end_matches("/**");
                let folder = folder
                    .strip_prefix("~/")
                    .map_or_else(|| folder.to_string(), |rest| format!("{home}/{rest}"));

                workspace == folder || workspace.starts_with(&format!("{folder}/"))
            })
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

        self.validate_scope_and_skills()?;
        self.rules
            .iter()
            .try_for_each(|rule| self.validate_rule(rule))
    }

    fn validate_scope_and_skills(&self) -> Result<(), String> {
        let scoped = |pattern: &String| {
            (pattern.starts_with('/') || pattern.starts_with("~/"))
                && !pattern.contains("..")
                && !pattern.trim_end_matches("/**").contains('*')
        };

        if self.scope.len() > MAX_SCOPES || !self.scope.iter().all(scoped) {
            return Err(format!(
                "{}: scope lists at most {MAX_SCOPES} absolute or ~/ folders, each optionally ending in /**",
                self.id
            ));
        }

        let input = [&self.input_skills.jira, &self.input_skills.slack];
        let skills: Vec<&String> = self
            .skills
            .iter()
            .chain(input.into_iter().flatten())
            .collect();

        if self.skills.len() > MAX_SKILLS
            || !skills.iter().all(|skill| crate::rule::identifier(skill))
        {
            return Err(format!(
                "{}: skills lists at most {MAX_SKILLS} skill names",
                self.id
            ));
        }

        Ok(())
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
    parse_all(read_json_files(directory, MAX_RULEBOOKS, MAX_FILE_BYTES)?)
}

/// A project's own rulebooks for `workspace`: `.chauffeur/rulebooks/` from
/// its Git root down to the workspace, as for project rules. They are
/// offered only there, so their `scope` is not consulted.
///
/// # Errors
///
/// An unreadable or invalid rulebook, or a repeated ID.
pub fn load_project_rulebooks(workspace: &str) -> Result<Vec<Rulebook>, String> {
    parse_all(crate::source::project_json(
        workspace,
        "rulebooks",
        MAX_FILE_BYTES,
    )?)
}

fn parse_all(files: Vec<(std::path::PathBuf, Vec<u8>)>) -> Result<Vec<Rulebook>, String> {
    let mut books: Vec<Rulebook> = Vec::new();

    for (path, bytes) in files {
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
