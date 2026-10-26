//! The judge interface. A judge is whatever answers what a capability asks:
//! System One (a fast classifier such as Jev), System Two (an LLM that
//! reasons over evidence), or plain code that decides from facts. The engine
//! hands each question and reasoning to one judge by name and reads every
//! judge's rulings the same way, so a capability never knows which kind
//! answered, and configuration can move a question from one judge to another.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use crate::system_one::{Answer, Question, SystemOne};
use crate::system_two::{self, Reasoning, ReasoningAttempt, ReasoningFailure, SystemTwo};

/// The judge a question goes to unless a route names another.
pub const SYSTEM_ONE: &str = "system_one";
/// The judge a reasoning goes to unless a route names another.
pub const SYSTEM_TWO: &str = "system_two";

/// Named whole numbers a question carries, such as `idle_seconds`.
pub type Facts = BTreeMap<String, i64>;

/// What one judge is asked in one call.
#[derive(Clone, Copy, Debug)]
pub struct Docket<'a> {
    /// The agent's recent activity, redacted. A judge that decides from
    /// facts ignores it.
    pub state: &'a str,
    pub questions: &'a [Question],
    pub reasonings: &'a [Reasoning],
    /// Time left for the whole call.
    pub timeout: Duration,
}

/// A judge's answers to one docket.
#[derive(Debug)]
pub struct Rulings {
    /// Answers to the docket's questions, or why the call failed. A question
    /// left unanswered reads as unanswered.
    pub answers: Result<Vec<Answer>, String>,
    /// Each reasoning's attempt, by request ID.
    pub reasoned: Vec<(String, ReasoningAttempt)>,
}

impl Rulings {
    /// Every ask in `docket` failed for `why`.
    #[must_use]
    pub fn failed(docket: &Docket<'_>, why: &str) -> Self {
        Self {
            answers: if docket.questions.is_empty() {
                Ok(Vec::new())
            } else {
                Err(why.to_string())
            },
            reasoned: unavailable(docket.reasonings, why),
        }
    }
}

pub trait Judge: Send {
    /// The name routes refer to, such as [`SYSTEM_ONE`].
    fn name(&self) -> &str;

    /// Whether the judge decides in place, without a model or other I/O. The
    /// engine rules with a local judge while it plans a round; every other
    /// judge's asks go to the engine's host.
    fn local(&self) -> bool {
        false
    }

    /// Answer every question and reasoning in `docket`, in one call.
    fn judge(&mut self, docket: &Docket<'_>) -> Rulings;
}

impl Judge for Box<dyn Judge> {
    fn name(&self) -> &str {
        (**self).name()
    }

    fn local(&self) -> bool {
        (**self).local()
    }

    fn judge(&mut self, docket: &Docket<'_>) -> Rulings {
        (**self).judge(docket)
    }
}

/// System One as a judge: a docket's questions in one call. It does not
/// reason.
pub struct SystemOneJudge(pub Box<dyn SystemOne>);

impl Judge for SystemOneJudge {
    fn name(&self) -> &str {
        SYSTEM_ONE
    }

    fn judge(&mut self, docket: &Docket<'_>) -> Rulings {
        Rulings {
            answers: if docket.questions.is_empty() {
                Ok(Vec::new())
            } else {
                self.0
                    .ask(docket.state, docket.questions)
                    .map_err(|error| error.0)
            },
            reasoned: unavailable(docket.reasonings, "System One does not reason"),
        }
    }
}

/// System Two as a judge: each of a docket's reasonings in turn, within the
/// docket's time. It answers no questions.
pub struct SystemTwoJudge(pub Box<dyn SystemTwo>);

impl Judge for SystemTwoJudge {
    fn name(&self) -> &str {
        SYSTEM_TWO
    }

    fn judge(&mut self, docket: &Docket<'_>) -> Rulings {
        let started = Instant::now();

        Rulings {
            answers: if docket.questions.is_empty() {
                Ok(Vec::new())
            } else {
                Err("System Two answers reasonings, not questions".into())
            },
            reasoned: docket
                .reasonings
                .iter()
                .map(|request| {
                    let left = docket.timeout.saturating_sub(started.elapsed());

                    (
                        request.id.clone(),
                        system_two::run_attempt(Some(self.0.as_mut()), request, left),
                    )
                })
                .collect(),
        }
    }
}

/// Every reasoning in `reasonings`, unavailable for `why`.
#[must_use]
pub fn unavailable(reasonings: &[Reasoning], why: &str) -> Vec<(String, ReasoningAttempt)> {
    reasonings
        .iter()
        .map(|request| {
            (
                request.id.clone(),
                Err(ReasoningFailure::Unavailable(why.to_string())).into(),
            )
        })
        .collect()
}

/// Which judge answers each ask, by its namespaced ID, such as
/// `model-router/renew_context`. An ask without a route goes to
/// [`SYSTEM_ONE`] for a question and [`SYSTEM_TWO`] for a reasoning.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Routes(HashMap<String, String>);

impl Routes {
    #[must_use]
    pub fn new(routes: HashMap<String, String>) -> Self {
        Self(routes)
    }

    /// The judge for question `id`.
    #[must_use]
    pub fn question(&self, id: &str) -> &str {
        self.0.get(id).map_or(SYSTEM_ONE, String::as_str)
    }

    /// The judge for reasoning `id`.
    #[must_use]
    pub fn reasoning(&self, id: &str) -> &str {
        self.0.get(id).map_or(SYSTEM_TWO, String::as_str)
    }

    /// Every judge a route names.
    pub fn judges(&self) -> impl Iterator<Item = &str> {
        self.0.values().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system_one::{AnswerValue, QuestionKind, SystemOneError};

    struct Yes;

    impl SystemOne for Yes {
        fn name(&self) -> &str {
            "jev"
        }

        fn ask(&mut self, _: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
            Ok(questions
                .iter()
                .map(|question| Answer {
                    id: question.id.clone(),
                    value: AnswerValue::Noul(1.0),
                    confidence: None,
                })
                .collect())
        }
    }

    fn question(id: &str) -> Question {
        Question {
            id: id.into(),
            instructions: "?".into(),
            kind: QuestionKind::Noul,
            facts: Facts::new(),
        }
    }

    #[test]
    fn system_one_is_a_judge_named_for_its_role_not_its_provider() {
        let mut judge = SystemOneJudge(Box::new(Yes));
        let questions = [question("a")];
        let rulings = judge.judge(&Docket {
            state: "",
            questions: &questions,
            reasonings: &[],
            timeout: Duration::from_secs(1),
        });

        assert_eq!(judge.name(), SYSTEM_ONE);
        assert!(!judge.local());
        assert_eq!(rulings.answers.unwrap()[0].id, "a");
    }

    #[test]
    fn an_ask_goes_to_its_route_or_its_kind_s_default() {
        let routes = Routes::new(HashMap::from([(
            "router/renew".to_string(),
            "facts".to_string(),
        )]));

        assert_eq!(routes.question("router/renew"), "facts");
        assert_eq!(routes.question("router/other"), SYSTEM_ONE);
        assert_eq!(routes.reasoning("rules/review"), SYSTEM_TWO);
    }
}
