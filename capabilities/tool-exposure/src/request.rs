//! The agent's own request for a tool. System One picks the one hidden tool
//! group or Code Mode namespace that serves it, or none; a hidden group is
//! revealed, and a namespace's best matches reach the running turn.

use chauffeur_core::{ChoiceOption, CodeModeNamespace, Question, QuestionKind};

/// The request's question ID.
pub const ID: &str = "request";
pub const NONE: &str = "none";
const GROUP: &str = "tools:";
const NAMESPACE: &str = "code-mode:";
/// Options offered, in host order: hidden groups first, then namespaces.
const MAX_OPTIONS: usize = 64;

/// What the agent's request may be granted.
pub enum Pick<'a> {
    Group(&'a str),
    Namespace(&'a str),
}

/// Parse a chosen option back into what it grants.
#[must_use]
pub fn pick(choice: &str) -> Option<Pick<'_>> {
    choice
        .strip_prefix(GROUP)
        .map(Pick::Group)
        .or_else(|| choice.strip_prefix(NAMESPACE).map(Pick::Namespace))
}

/// The question for `need`, or `None` when nothing could serve it.
#[must_use]
pub fn question(
    groups: &[(String, String)],
    namespaces: &[CodeModeNamespace],
    need: &str,
    user_request: &str,
) -> Option<Question> {
    let mut options: Vec<ChoiceOption> = groups
        .iter()
        .map(|(name, listing)| ChoiceOption {
            value: format!("{GROUP}{name}"),
            description: format!("Hidden direct tools: {listing}"),
        })
        .chain(namespaces.iter().map(|namespace| ChoiceOption {
            value: format!("{NAMESPACE}{}", namespace.name),
            description: format!(
                "{} Code Mode tools reached through execute, such as: {}",
                namespace.size,
                namespace
                    .tools
                    .iter()
                    .map(|tool| tool.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }))
        .take(MAX_OPTIONS)
        .collect();

    if options.is_empty() {
        return None;
    }

    options.push(ChoiceOption {
        value: NONE.into(),
        description: "Nothing offered serves the request.".into(),
    });

    Some(Question {
        id: ID.into(),
        instructions: format!(
            "The coding agent asked for a tool it lacks: \"{need}\". The user's latest request: \
             {user_request}. Which offered tools serve what the agent asked for? Choose none \
             unless an option clearly does."
        ),
        kind: QuestionKind::Choice { options },
    })
}
