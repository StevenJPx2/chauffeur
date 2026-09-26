//! A prompt's skill candidates: one yes/no question per offered skill, all in
//! one System One call, so a request gets every skill it needs. Exact facts
//! about the prompt choose a skill's wording and the rule its answer meets.

use chauffeur_core::judge::strategy::Candidate;
use chauffeur_core::{CatalogEntry, Question, QuestionKind, Rule, Signal, SignalKind};

/// A skill attaches on P(needed) ≥ 0.7 with confidence ≥ 0.4.
pub const NEEDED: Rule = Rule::yes(0.7, 0.4);
/// A project's skill applies in its project unless Jev confidently says the
/// request is about something else.
pub const IN_PROJECT: Rule = Rule::unless_no(0.3, 0.4);

/// Why a skill is asked about the way it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Basis {
    /// No fact points at it: does the request need it?
    Offered,
    /// Its leading word is a word of the request, such as `slack` in
    /// `adeptmind.slack.com` for `slack-cli`.
    Named,
    /// Named after a directory the session works in, such as `hpdp-overlay`
    /// for `…/hpdp-overlay/ADEPT-45130`.
    Project,
}

impl Basis {
    fn prefix(self) -> &'static str {
        match self {
            Self::Offered => "skill:",
            Self::Named => "named:",
            Self::Project => "project:",
        }
    }

    fn rule(self) -> Rule {
        match self {
            Self::Project => IN_PROJECT,
            Self::Offered | Self::Named => NEEDED,
        }
    }
}

/// A candidate for each offerable skill, keyed by skill ID.
#[must_use]
pub fn candidates(signal: &Signal, offerable: &[&CatalogEntry]) -> Vec<Candidate<String>> {
    let facts = Facts::of(signal);

    offerable
        .iter()
        .map(|skill| {
            let basis = facts.basis(skill);

            Candidate {
                key: skill.id.clone(),
                question: Question {
                    id: format!("{}{}", basis.prefix(), skill.id),
                    instructions: instructions(signal, skill, basis),
                    kind: QuestionKind::Noul,
                },
                rule: basis.rule(),
            }
        })
        .collect()
}

/// The prompt's words and the session's directories, lowercased.
struct Facts {
    said: Vec<String>,
    directories: Vec<String>,
}

impl Facts {
    fn of(signal: &Signal) -> Self {
        let (text, workspace) = match &signal.kind {
            SignalKind::UserMessage {
                text, workspace, ..
            } => (text.as_str(), workspace.as_str()),
            // An agent's request is judged on what it asked for alone.
            _ => ("", ""),
        };

        Self {
            said: words(&text.replace('-', " ")),
            directories: words(workspace),
        }
    }

    fn basis(&self, skill: &CatalogEntry) -> Basis {
        let id = skill.id.to_ascii_lowercase();
        let lead = id.split('-').next().unwrap_or_default();

        if self.directories.contains(&id) {
            Basis::Project
        } else if lead.len() >= 3 && self.said.iter().any(|word| word == lead) {
            Basis::Named
        } else {
            Basis::Offered
        }
    }
}

fn words(value: &str) -> Vec<String> {
    value
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
        .filter(|word| !word.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn instructions(signal: &Signal, skill: &CatalogEntry, basis: Basis) -> String {
    let (id, description) = (&skill.id, &skill.description);

    match (&signal.kind, basis) {
        (
            SignalKind::AgentRequest {
                need, user_request, ..
            },
            _,
        ) => format!(
            "The coding agent asked for: \"{need}\" (the user's latest request: {user_request}). \
             Does the {id} skill ({description}) serve what it asked for? Answer yes only when \
             it clearly does."
        ),
        (_, Basis::Project) => format!(
            "The coding session works inside the {id} project ({}). The {id} skill: \
             {description}. Does the user's latest request involve work in this project that \
             the skill covers? Answer no only when the request is clearly about something else.",
            workspace(signal)
        ),
        (_, Basis::Named) => format!(
            "The user's latest request mentions {}. The {id} skill: {description}. Does the \
             agent need this skill to act on the request?",
            id.split('-').next().unwrap_or_default()
        ),
        (_, Basis::Offered) => format!(
            "The coding session works in {}. Does the user's latest request need the {id} skill \
             ({description}) for the agent to act on it? Answer yes only when the skill clearly \
             helps with this request.",
            workspace(signal)
        ),
    }
}

fn workspace(signal: &Signal) -> &str {
    match &signal.kind {
        SignalKind::UserMessage { workspace, .. } if !workspace.is_empty() => workspace,
        _ => "an unknown directory",
    }
}
