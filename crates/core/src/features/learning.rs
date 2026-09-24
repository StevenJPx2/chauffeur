//! Core questions that grow the safety lists. On a shell permission request,
//! System One is asked whether the command is irreversibly destructive; a
//! confident yes is denied and learned by the backstop. Each credential-like
//! shape found while redacting is asked about, never its value; the answer
//! is learned by the redactor.

use crate::backstop::Backstop;
use crate::redact::{Masking, Redactor, Shape, describe};
use crate::signal::{Signal, SignalKind};
use crate::system_one::{Answer, AnswerValue, Question, QuestionKind};

const HARM: &str = "core/irreversible";
const SECRET: &str = "core/secret-";
/// Shapes asked about per signal, at most.
const MAX_SHAPES: usize = 4;
/// Learning changes safety lists, so it needs a clearer answer than acting.
const HARM_AT_OR_ABOVE: f32 = 0.9;
const SECRET_AT_OR_ABOVE: f32 = 0.7;
const SAFE_AT_OR_BELOW: f32 = 0.3;
const MIN_CONFIDENCE: f32 = 0.6;

/// What one signal asked the core, to learn from its answers.
#[derive(Debug, Default)]
pub struct Probe {
    command: Option<String>,
    shapes: Vec<Shape>,
}

/// Shell commands the backstop has not vetoed get the irreversible-harm
/// question.
#[must_use]
pub fn harm(signal: &Signal) -> Option<(String, Question)> {
    let SignalKind::PermissionRequest {
        action, resources, ..
    } = &signal.kind
    else {
        return None;
    };
    let command = resources
        .iter()
        .map(|resource| resource.requested.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    if !matches!(action.as_str(), "shell" | "bash") || command.trim().is_empty() {
        return None;
    }

    let question = Question {
        id: HARM.into(),
        instructions: format!(
            "Would running this exact command cause irreversible harm, such as deleting data \
             that cannot be recovered, wiping or overwriting a disk, rewriting shared history \
             (force-pushing over a shared branch), or dropping a production database? \
             Command: {command}"
        ),
        kind: QuestionKind::Noul,
    };

    Some((command, question))
}

/// One question per unknown shape found while redacting, at most
/// [`MAX_SHAPES`]; each names the shape, never the value.
#[must_use]
pub fn secrets(masking: &Masking) -> Vec<Question> {
    masking
        .shapes
        .iter()
        .take(MAX_SHAPES)
        .enumerate()
        .map(|(index, (shape, key))| Question {
            id: format!("{SECRET}{}", index + 1),
            instructions: format!(
                "The state shows {} in place of a string. Going by its shape and where it \
                 appears, is that string a secret credential such as an API key, access token, \
                 or password, rather than an ID, hash, or name?",
                describe(index + 1, shape, key)
            ),
            kind: QuestionKind::Noul,
        })
        .collect()
}

impl Probe {
    #[must_use]
    pub fn new(command: Option<String>, masking: &Masking) -> Self {
        Self {
            command,
            shapes: masking
                .shapes
                .iter()
                .take(MAX_SHAPES)
                .map(|(shape, _)| shape.clone())
                .collect(),
        }
    }

    /// Learn from the core answers. Returns what was learned, for the audit
    /// log, and whether the command was judged irreversible.
    pub fn learn(
        &self,
        answers: &[Answer],
        backstop: &mut Backstop,
        redactor: &mut Redactor,
    ) -> (Vec<String>, bool) {
        let mut learned = Vec::new();
        let irreversible = self
            .command
            .as_ref()
            .is_some_and(|_| confident(answers, HARM, |p| p >= HARM_AT_OR_ABOVE));

        if let Some(command) = self.command.as_ref().filter(|_| irreversible) {
            if backstop.learn(command) {
                learned.push(format!("backstop: {command}"));
            }
        }

        for (index, shape) in self.shapes.iter().enumerate() {
            let id = format!("{SECRET}{}", index + 1);
            let secret = confident(answers, &id, |p| p >= SECRET_AT_OR_ABOVE);
            let safe = confident(answers, &id, |p| p <= SAFE_AT_OR_BELOW);
            let label = describe(index + 1, shape, "");

            if (secret || safe) && redactor.learn(shape.clone(), secret) {
                learned.push(format!(
                    "{} shape: {label}",
                    if secret { "secret" } else { "safe" }
                ));
            }
        }

        (learned, irreversible)
    }
}

/// Whether the core question `id` is a confident noul that `accept` takes.
fn confident(answers: &[Answer], id: &str, accept: impl Fn(f32) -> bool) -> bool {
    answers.iter().any(|answer| {
        answer.id == id
            && matches!(answer.value, AnswerValue::Noul(p) if accept(p))
            && answer.effective_confidence() >= MIN_CONFIDENCE
    })
}
