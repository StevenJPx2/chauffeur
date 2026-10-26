//! A prompt's skill candidates: one yes/no question per offered skill, all in
//! one System One call, so a request gets every skill it needs. Exact facts
//! about the prompt choose a skill's wording and the rule its answer meets.

use chauffeur_core::judgment::strategy::Candidate;
use chauffeur_core::{CatalogEntry, Question, QuestionKind, Rule, Signal, SignalKind};

use crate::{SkillExposureConfig, SkillTexts};

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

    /// A project's skill applies in its project unless Jev confidently says
    /// the request is about something else; any other skill needs a yes.
    fn rule(self, config: &SkillExposureConfig) -> Rule {
        match self {
            Self::Project => config.in_project.unless_no(),
            Self::Offered | Self::Named => config.needed.yes(),
        }
    }
}

/// A skill's place in `ranking`; unranked skills sort last, in host order.
pub(crate) fn rank(ranking: &[String], id: &str) -> usize {
    ranking
        .iter()
        .position(|ranked| ranked == id)
        .unwrap_or(usize::MAX)
}

/// The skills a prompt or request judges. Pinned skills always: the
/// configured `always` ones, ones the request names, and the project's own.
/// With the daemon's ranking, the closest `judged` others follow; without
/// it, every other skill in order, up to the cap.
pub(crate) fn judged<'a>(
    signal: &Signal,
    offerable: &[&'a CatalogEntry],
    config: &SkillExposureConfig,
) -> Vec<&'a CatalogEntry> {
    let ranking: &[String] = match &signal.kind {
        SignalKind::UserMessage { ranking, .. } | SignalKind::AgentRequest { ranking, .. } => {
            ranking
        }
        _ => &[],
    };
    let facts = Facts::of(signal);
    let pinned = |skill: &CatalogEntry| {
        config.ranking.always.contains(&skill.id) || facts.basis(skill) != Basis::Offered
    };
    let (mut chosen, mut rest): (Vec<&CatalogEntry>, Vec<&CatalogEntry>) =
        offerable.iter().copied().partition(|skill| pinned(skill));

    if !ranking.is_empty() {
        rest.retain(|skill| ranking.contains(&skill.id));
        rest.sort_by_key(|skill| rank(ranking, &skill.id));
        rest.truncate(config.ranking.judged);
    }

    chosen.extend(rest);
    chosen.truncate(crate::MAX_OPTIONS);
    chosen
}

/// A candidate for each offerable skill, keyed by skill ID.
#[must_use]
pub fn candidates(
    signal: &Signal,
    offerable: &[&CatalogEntry],
    config: &SkillExposureConfig,
) -> Vec<Candidate<String>> {
    let facts = Facts::of(signal);

    offerable
        .iter()
        .map(|skill| {
            let basis = facts.basis(skill);

            Candidate {
                key: skill.id.clone(),
                question: Question {
                    id: format!("{}{}", basis.prefix(), skill.id),
                    instructions: instructions(signal, skill, basis, &config.texts),
                    kind: QuestionKind::Noul,
                    facts: chauffeur_core::Facts::new(),
                },
                rule: basis.rule(config),
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

fn instructions(signal: &Signal, skill: &CatalogEntry, basis: Basis, texts: &SkillTexts) -> String {
    let (id, description) = (skill.id.as_str(), skill.description.as_str());

    match (&signal.kind, basis) {
        (
            SignalKind::AgentRequest {
                need, user_request, ..
            },
            _,
        ) => texts.request.render(&[
            ("need", need),
            ("user_request", user_request),
            ("skill", id),
            ("description", description),
        ]),
        (_, Basis::Project) => texts.project.render(&[
            ("skill", id),
            ("description", description),
            ("workspace", workspace(signal, texts)),
        ]),
        (_, Basis::Named) => texts.named.render(&[
            ("word", id.split('-').next().unwrap_or_default()),
            ("skill", id),
            ("description", description),
        ]),
        (_, Basis::Offered) => texts.offered.render(&[
            ("skill", id),
            ("description", description),
            ("workspace", workspace(signal, texts)),
        ]),
    }
}

fn workspace<'a>(signal: &'a Signal, texts: &'a SkillTexts) -> &'a str {
    match &signal.kind {
        SignalKind::UserMessage { workspace, .. } if !workspace.is_empty() => workspace,
        _ => texts.unknown_workspace.as_str(),
    }
}
