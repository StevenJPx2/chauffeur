//! The agent's own request for a tool. System One judges every hidden tool
//! group and Code Mode namespace against what the agent asked for, in one
//! call; each one that covers any part of it is granted, since a request
//! can need several: hidden groups are revealed, and
//! the namespaces' best matches reach the running turn.

use chauffeur_core::judge::strategy::Candidate;
use chauffeur_core::{CodeModeNamespace, Question, QuestionKind, Rule};

use crate::config::ToolTexts;
use crate::{Grant, code_mode};

/// Candidates judged, in host order: hidden groups first, then namespaces.
const MAX_CANDIDATES: usize = 64;

/// One candidate per hidden group, given as `(name, listing)`, and per
/// namespace, each granted when `needed` holds; none when nothing could
/// serve the request.
#[must_use]
pub fn candidates(
    groups: &[(String, String)],
    namespaces: &[CodeModeNamespace],
    need: &str,
    user_request: &str,
    needed: Rule,
    texts: &ToolTexts,
) -> Vec<Candidate<Grant>> {
    let asked = texts
        .request_asked
        .render(&[("need", need), ("user_request", user_request)]);
    let groups = groups.iter().map(|(name, listing)| Candidate {
        key: Grant::Group(name.clone()),
        question: Question {
            id: format!("tools:{name}"),
            instructions: texts.request_group.render(&[
                ("asked", &asked),
                ("group", name),
                ("tools", listing),
            ]),
            kind: QuestionKind::Noul,
        },
        rule: needed,
    });
    let namespaces = namespaces.iter().map(|namespace| Candidate {
        key: Grant::Namespace(namespace.name.clone()),
        question: Question {
            id: code_mode::question_id(&namespace.name),
            instructions: texts.request_namespace.render(&[
                ("asked", &asked),
                ("namespace", &namespace.name),
                ("size", &namespace.size.to_string()),
                ("examples", &code_mode::examples(namespace)),
            ]),
            kind: QuestionKind::Noul,
        },
        rule: needed,
    });

    groups.chain(namespaces).take(MAX_CANDIDATES).collect()
}
