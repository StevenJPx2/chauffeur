//! Core questions that grow the safety lists. On a shell permission request,
//! System One is asked whether the command is irreversibly destructive; a
//! confident yes is denied and learned by the backstop. Each credential-like
//! shape found while redacting is asked about, never its value; the answer
//! is learned by the redactor.

use std::path::Path;

use serde::Deserialize;

use crate::backstop::Backstop;
use crate::config::load_layered;
use crate::judge::Threshold;
use crate::redact::{Masking, Redactor, Shape, describe};
use crate::signal::{Signal, SignalKind};
use crate::system_one::{Answer, Question, QuestionKind};

const HARM: &str = "core/irreversible";
const SECRET: &str = "core/secret-";
/// Shapes asked about per signal, at most.
const MAX_SHAPES: usize = 4;
/// The shipped bars (`skills/safety/learning.json`), compiled in.
const SHIPPED: &str = include_str!("../../../../skills/safety/learning.json");

/// When an answer changes a safety list. Learning outlives the signal, so
/// its bars are stricter than acting's.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LearningConfig {
    /// A command this surely irreversible is denied and joins the backstop.
    pub irreversible: Threshold,
    /// A shape this surely a secret is always redacted from then on.
    pub secret: Threshold,
    /// A shape this surely not a secret passes where it appeared.
    pub safe: Threshold,
}

impl LearningConfig {
    /// The shipped bars overlaid by your `learning.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, or a bar is outside `[0, 1]`.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered(SHIPPED, path)
    }
}

impl Default for LearningConfig {
    fn default() -> Self {
        serde_json::from_str(SHIPPED).expect("shipped learning bars are valid")
    }
}

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
        config: &LearningConfig,
        answers: &[Answer],
        backstop: &mut Backstop,
        redactor: &mut Redactor,
    ) -> (Vec<String>, bool) {
        let answer = |id: &str| answers.iter().find(|answer| answer.id == id);
        let mut learned = Vec::new();
        let irreversible = self.command.is_some() && config.irreversible.yes().holds(answer(HARM));

        if let Some(command) = self.command.as_ref().filter(|_| irreversible) {
            if backstop.learn(command) {
                learned.push(format!("backstop: {command}"));
            }
        }

        for (index, shape) in self.shapes.iter().enumerate() {
            let id = format!("{SECRET}{}", index + 1);
            let secret = config.secret.yes().holds(answer(&id));
            let safe = config.safe.no().holds(answer(&id));
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
