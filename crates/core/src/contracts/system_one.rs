//! Provider-neutral System One interface: typed questions in, typed answers out.

use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChoiceOption {
    pub value: String,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionKind {
    Choice {
        options: Vec<ChoiceOption>,
    },
    /// Ordered levels, lowest first.
    Score {
        levels: Vec<String>,
    },
    /// Yes/no; the answer is P(yes).
    Noul,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub instructions: String,
    pub kind: QuestionKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AnswerValue {
    Choice(String),
    /// Probability-weighted level index in `[0, levels - 1]`.
    Score(f32),
    /// P(yes) in `[0, 1]`.
    Noul(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Answer {
    pub id: String,
    pub value: AnswerValue,
    /// Provider-reported confidence. Some providers omit it for `Noul`.
    pub confidence: Option<f32>,
}

impl Answer {
    /// Confidence used for thresholds. For a `Noul` without reported
    /// confidence, the margin from an even split, `|2p - 1|`.
    #[must_use]
    pub fn effective_confidence(&self) -> f32 {
        match (self.confidence, &self.value) {
            (Some(confidence), _) => confidence,
            (None, AnswerValue::Noul(probability)) => (2.0 * probability - 1.0).abs(),
            (None, _) => 0.0,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct SystemOneError(pub String);

impl fmt::Display for SystemOneError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "system one failed: {}", self.0)
    }
}

impl std::error::Error for SystemOneError {}

/// A typed-decision model. Implementations answer every question in one call
/// and return answers only after [`validate_answers`] accepts them.
pub trait SystemOne: Send {
    fn name(&self) -> &str;

    fn ask(&mut self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError>;
}

impl SystemOne for Box<dyn SystemOne> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn ask(&mut self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        (**self).ask(state, questions)
    }
}

/// Check that answers match the questions exactly, one each, in range.
pub fn validate_answers(questions: &[Question], answers: &[Answer]) -> Result<(), SystemOneError> {
    if answers.len() != questions.len() {
        return Err(SystemOneError(format!(
            "expected {} answers, got {}",
            questions.len(),
            answers.len()
        )));
    }

    for question in questions {
        let answer = answers
            .iter()
            .find(|answer| answer.id == question.id)
            .ok_or_else(|| SystemOneError(format!("missing answer {}", question.id)))?;

        validate_answer(question, answer)?;
    }

    Ok(())
}

fn validate_answer(question: &Question, answer: &Answer) -> Result<(), SystemOneError> {
    if let Some(confidence) = answer.confidence {
        check_range(confidence, 1.0, &question.id, "confidence")?;
    }

    match (&question.kind, &answer.value) {
        (QuestionKind::Choice { options }, AnswerValue::Choice(choice)) => {
            if options.iter().any(|option| option.value == *choice) {
                Ok(())
            } else {
                Err(SystemOneError(format!(
                    "{}: unknown choice {choice}",
                    question.id
                )))
            }
        }
        (QuestionKind::Score { levels }, AnswerValue::Score(score)) => {
            let top = u16::try_from(levels.len().saturating_sub(1))
                .map_err(|_| SystemOneError(format!("{}: too many levels", question.id)))?;

            check_range(*score, f32::from(top), &question.id, "score")
        }
        (QuestionKind::Noul, AnswerValue::Noul(probability)) => {
            check_range(*probability, 1.0, &question.id, "noul")
        }
        _ => Err(SystemOneError(format!(
            "{}: answer type mismatch",
            question.id
        ))),
    }
}

fn check_range(value: f32, max: f32, id: &str, field: &str) -> Result<(), SystemOneError> {
    if value.is_finite() && (0.0..=max).contains(&value) {
        Ok(())
    } else {
        Err(SystemOneError(format!(
            "{id}: {field} {value} outside [0, {max}]"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noul(p: f32) -> Answer {
        Answer {
            id: "q".into(),
            value: AnswerValue::Noul(p),
            confidence: None,
        }
    }

    #[test]
    fn noul_confidence_is_the_margin_from_even() {
        assert!((noul(0.9).effective_confidence() - 0.8).abs() < 1e-6);
        assert!((noul(0.5).effective_confidence()).abs() < 1e-6);
    }

    #[test]
    fn answers_must_match_questions() {
        let question = Question {
            id: "q".into(),
            instructions: "i".into(),
            kind: QuestionKind::Choice {
                options: vec![
                    ChoiceOption {
                        value: "a".into(),
                        description: String::new(),
                    },
                    ChoiceOption {
                        value: "b".into(),
                        description: String::new(),
                    },
                ],
            },
        };
        let good = Answer {
            id: "q".into(),
            value: AnswerValue::Choice("a".into()),
            confidence: Some(0.7),
        };
        let unknown = Answer {
            id: "q".into(),
            value: AnswerValue::Choice("z".into()),
            confidence: None,
        };

        assert!(validate_answers(std::slice::from_ref(&question), &[good]).is_ok());
        assert!(validate_answers(std::slice::from_ref(&question), &[unknown]).is_err());
        assert!(validate_answers(&[question], &[noul(0.5)]).is_err());
    }
}
