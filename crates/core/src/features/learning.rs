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
use crate::template::Template;

const HARM: &str = "core/irreversible";
const SECRET: &str = "core/secret-";
/// Shapes asked about per signal, at most.
const MAX_SHAPES: usize = 4;
/// The shipped bars (`skills/safety/learning.json`), compiled in.
const SHIPPED: &str = include_str!("../../../../skills/safety/learning.json");

/// When an answer changes a safety list. Learning outlives the signal, so
/// its bars are stricter than acting's.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LearningConfig {
    /// A command this surely irreversible is denied and joins the backstop.
    pub irreversible: Threshold,
    /// A shape this surely a secret is always redacted from then on.
    pub secret: Threshold,
    /// A shape this surely not a secret passes where it appeared.
    pub safe: Threshold,
    /// The wording of the questions to System One.
    pub texts: LearningTexts,
}

/// The core's two questions to System One.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LearningTexts {
    /// Whether a shell command is irreversibly harmful: `{command}`.
    pub irreversible: Template,
    /// Whether a redacted string is a secret: `{shape}`, how the state names it.
    pub secret: Template,
}

impl LearningTexts {
    fn checked(self) -> Result<Self, String> {
        self.irreversible
            .check("texts.irreversible", &["command"])?;
        self.secret.check("texts.secret", &["shape"])?;

        Ok(self)
    }
}

impl LearningConfig {
    /// The shipped bars and texts overlaid by your `learning.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable, invalid, a bar is outside `[0, 1]`, or a
    /// text names a placeholder it may not.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered::<Self>(SHIPPED, path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    fn checked(mut self) -> Result<Self, String> {
        self.texts = self.texts.checked()?;

        Ok(self)
    }
}

impl Default for LearningConfig {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
            .expect("shipped learning defaults are valid")
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
pub fn harm(signal: &Signal, texts: &LearningTexts) -> Option<(String, Question)> {
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
        instructions: texts.irreversible.render(&[("command", &command)]),
        kind: QuestionKind::Noul,
    };

    Some((command, question))
}

/// One question per unknown shape found while redacting, at most
/// [`MAX_SHAPES`]; each names the shape, never the value.
#[must_use]
pub fn secrets(masking: &Masking, texts: &LearningTexts) -> Vec<Question> {
    masking
        .shapes
        .iter()
        .take(MAX_SHAPES)
        .enumerate()
        .map(|(index, (shape, key))| Question {
            id: format!("{SECRET}{}", index + 1),
            instructions: texts
                .secret
                .render(&[("shape", &describe(index + 1, shape, key))]),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::Resource;

    #[test]
    fn the_shipped_question_names_the_command() {
        let signal = Signal {
            agent_id: "a".into(),
            at: 1,
            kind: SignalKind::PermissionRequest {
                action: "shell".into(),
                resources: vec![Resource {
                    requested: "dd if=/dev/zero of=disk".into(),
                    resolved: "dd if=/dev/zero of=disk".into(),
                }],
                request: String::new(),
                workspace: String::new(),
                user_requests: Vec::new(),
                host_decision: crate::effect::PermissionDecision::Allow,
            },
        };
        let (command, question) = harm(&signal, &LearningConfig::default().texts).unwrap();

        assert_eq!(command, "dd if=/dev/zero of=disk");
        assert_eq!(
            question.instructions,
            "Would running this exact command cause irreversible harm, such as deleting data \
             that cannot be recovered, wiping or overwriting a disk, rewriting shared history \
             (force-pushing over a shared branch), or dropping a production database? \
             Command: dd if=/dev/zero of=disk"
        );
    }

    #[test]
    fn a_text_naming_an_unknown_placeholder_is_an_error() {
        let path =
            std::env::temp_dir().join(format!("chauffeur-learning-{}.json", std::process::id()));

        std::fs::write(
            &path,
            r#"{ "texts": { "secret": "Is {value} a secret?" } }"#,
        )
        .unwrap();

        let error = LearningConfig::load(&path).unwrap_err();

        assert!(error.contains("texts.secret names {value}"), "{error}");
        std::fs::remove_file(path).unwrap();
    }
}
