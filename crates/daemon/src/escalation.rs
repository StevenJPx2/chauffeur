//! Opt-in confidence escalation. Answers the first judge is unsure of are
//! asked again of a second System One model; answers it is unsure of too
//! may go to a System Two profile. Each step is best effort: a failed or
//! late step keeps the answers already in hand, so escalation never fails a
//! judgment the first judge answered.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use chauffeur_core::system_two::{self, MAX_REASONING_INPUT_BYTES};
use chauffeur_core::{
    Answer, AnswerValue, Evidence, Question, QuestionKind, Reasoning, Schema, SystemOne,
    SystemOneError, SystemTwo, validate_answers,
};
use serde_json::Value;

/// Time a second System One call needs to be worth starting.
const MIN_SECOND: Duration = Duration::from_millis(500);
/// Time a review needs to be worth starting.
const MIN_REVIEW: Duration = Duration::from_secs(2);
/// Kept back from a review for the engine's next round.
const ROUND_RESERVE: Duration = Duration::from_secs(1);
/// Questions one review answers at most: System Two's object field limit.
const MAX_REVIEWED: usize = 16;
/// Values a review field may hold, its abstention included.
const MAX_VALUES: usize = 32;
const UNSURE: &str = "unsure";
const STATE: &str = "state";
const OMITTED: &str = "(earlier activity omitted)\n";

/// The System Two step: its executor and the profile it runs.
pub(crate) struct Review {
    pub(crate) executor: Box<dyn SystemTwo>,
    pub(crate) profile: String,
}

pub(crate) struct Escalate {
    first: Box<dyn SystemOne>,
    second: Box<dyn SystemOne>,
    /// Answers with less effective confidence than this escalate.
    below: f32,
    review: Option<Review>,
    last_used: String,
    /// The first judge's answers that the last ask replaced.
    replaced: Vec<Answer>,
}

impl Escalate {
    pub(crate) fn new(
        first: Box<dyn SystemOne>,
        second: Box<dyn SystemOne>,
        below: f32,
        review: Option<Review>,
    ) -> Self {
        let last_used = first.name().to_string();

        Self {
            first,
            second,
            below,
            review,
            last_used,
            replaced: Vec::new(),
        }
    }

    /// Ask the first judge alone, for a caller that cannot wait for doubt
    /// to be settled, such as a permission request holding a tool call.
    pub(crate) fn ask_first(
        &mut self,
        state: &str,
        questions: &[Question],
        deadline: Instant,
    ) -> Result<Vec<Answer>, SystemOneError> {
        self.replaced.clear();
        let answers = ask(self.first.as_mut(), state, questions, Some(deadline));

        self.last_used = self.first.name().to_string();
        answers
    }

    /// The first judge's answers that the last ask replaced.
    pub(crate) fn take_replaced(&mut self) -> Vec<Answer> {
        std::mem::take(&mut self.replaced)
    }

    fn judge(
        &mut self,
        state: &str,
        questions: &[Question],
        deadline: Option<Instant>,
    ) -> Result<Vec<Answer>, SystemOneError> {
        self.replaced.clear();
        let mut answers = ask(self.first.as_mut(), state, questions, deadline)?;
        let mut used = self.first.name().to_string();
        let unsure = self.unsure(questions, &answers);

        if !unsure.is_empty() && has_time(deadline, MIN_SECOND) {
            match ask(self.second.as_mut(), state, &unsure, deadline) {
                Ok(second) => {
                    replace(&mut answers, second, &mut self.replaced);
                    used = format!("{used}+{}", self.second.name());
                }
                Err(error) => eprintln!(
                    "chauffeur: {} escalation failed; keeping {} answers: {}",
                    self.second.name(),
                    self.first.name(),
                    error.0
                ),
            }
        }

        let unsure = self.unsure(questions, &answers);

        if let Some(review) = self.review.as_mut().filter(|_| !unsure.is_empty()) {
            if has_time(deadline, MIN_REVIEW)
                && review_into(
                    review,
                    state,
                    &unsure,
                    deadline,
                    &mut answers,
                    &mut self.replaced,
                )
            {
                used = format!("{used}+system-two");
            }
        }

        self.last_used = used;
        Ok(answers)
    }

    fn unsure(&self, questions: &[Question], answers: &[Answer]) -> Vec<Question> {
        questions
            .iter()
            .filter(|question| {
                answers
                    .iter()
                    .find(|answer| answer.id == question.id)
                    .is_none_or(|answer| answer.effective_confidence() < self.below)
            })
            .cloned()
            .collect()
    }
}

impl SystemOne for Escalate {
    fn name(&self) -> &str {
        &self.last_used
    }

    fn ask(&mut self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        self.judge(state, questions, None)
    }

    fn ask_by(
        &mut self,
        state: &str,
        questions: &[Question],
        deadline: Instant,
    ) -> Result<Vec<Answer>, SystemOneError> {
        self.judge(state, questions, Some(deadline))
    }
}

fn ask(
    judge: &mut dyn SystemOne,
    state: &str,
    questions: &[Question],
    deadline: Option<Instant>,
) -> Result<Vec<Answer>, SystemOneError> {
    let answers = match deadline {
        Some(deadline) => judge.ask_by(state, questions, deadline)?,
        None => judge.ask(state, questions)?,
    };
    validate_answers(questions, &answers)?;

    Ok(answers)
}

fn has_time(deadline: Option<Instant>, needed: Duration) -> bool {
    deadline.is_none_or(|deadline| deadline.saturating_duration_since(Instant::now()) >= needed)
}

/// The escalated judge's answers stand, as benchmarked: it was asked only
/// because the first judge was unsure. Each answer replaced for the first
/// time is kept in `replaced`, so the first judge's own answer is on record.
fn replace(answers: &mut [Answer], better: Vec<Answer>, replaced: &mut Vec<Answer>) {
    for answer in better {
        if let Some(slot) = answers.iter_mut().find(|slot| slot.id == answer.id) {
            let original = std::mem::replace(slot, answer);

            if !replaced.iter().any(|kept| kept.id == original.id) {
                replaced.push(original);
            }
        }
    }
}

/// Ask the review profile; adopt each answer it gives. Whether it answered.
fn review_into(
    review: &mut Review,
    state: &str,
    questions: &[Question],
    deadline: Option<Instant>,
    answers: &mut [Answer],
    replaced: &mut Vec<Answer>,
) -> bool {
    let questions: Vec<&Question> = questions
        .iter()
        .filter(|question| values(question).len() <= MAX_VALUES)
        .take(MAX_REVIEWED)
        .collect();
    if questions.is_empty() {
        return false;
    }

    let request = request(&review.profile, state, &questions);
    let timeout = deadline.map_or(Duration::MAX, |deadline| {
        deadline
            .saturating_duration_since(Instant::now())
            .saturating_sub(ROUND_RESERVE)
    });
    let attempt = system_two::run_attempt(Some(review.executor.as_mut()), &request, timeout);

    match attempt.outcome {
        Ok(verdict) => {
            replace(answers, reviewed(&questions, &verdict), replaced);
            true
        }
        Err(error) => {
            eprintln!("chauffeur: System Two review failed; keeping answers: {error:?}");
            false
        }
    }
}

/// A question's allowed review values, abstention last.
fn values(question: &Question) -> Vec<String> {
    let mut values: Vec<String> = match &question.kind {
        QuestionKind::Noul => vec!["yes".into(), "no".into()],
        QuestionKind::Choice { options } => options.iter().map(|o| o.value.clone()).collect(),
        QuestionKind::Score { levels } => (0..levels.len()).map(|i| i.to_string()).collect(),
    };

    if !values.iter().any(|value| value == UNSURE) {
        values.push(UNSURE.into());
    }
    values
}

fn request(profile: &str, state: &str, questions: &[&Question]) -> Reasoning {
    let mut instructions = String::from(
        "Fast judges were unsure of these questions about an AI coding agent's session. \
         Answer each field, named by its question ID, from the evidence. \
         Answer unsure when the evidence does not settle it.\n",
    );

    for question in questions {
        let shape = match &question.kind {
            QuestionKind::Noul => "yes or no".to_string(),
            QuestionKind::Choice { options } => options
                .iter()
                .map(|o| format!("{} = {}", o.value, o.description))
                .collect::<Vec<_>>()
                .join("; "),
            QuestionKind::Score { levels } => levels
                .iter()
                .enumerate()
                .map(|(i, level)| format!("{i} = {level}"))
                .collect::<Vec<_>>()
                .join("; "),
        };
        let _ = write!(
            instructions,
            "\n{} ({shape}):\n{}\n",
            question.id, question.instructions
        );
    }

    let room = MAX_REASONING_INPUT_BYTES.saturating_sub(instructions.len() + STATE.len());

    Reasoning {
        id: "system-one-review".into(),
        profile: profile.into(),
        evidence: vec![Evidence {
            id: STATE.into(),
            text: tail(state, room),
        }],
        instructions,
        activity: false,
        schema: Schema::Object(
            questions
                .iter()
                .map(|question| (question.id.clone(), Schema::Enum(values(question))))
                .collect(),
        ),
    }
}

/// The most recent `room` bytes of `state`, marked when cut.
fn tail(state: &str, room: usize) -> String {
    if state.len() <= room {
        return state.into();
    }

    let mut start = state.len() - room.saturating_sub(OMITTED.len());
    while !state.is_char_boundary(start) {
        start += 1;
    }
    format!("{OMITTED}{}", &state[start..])
}

/// The verdict's answers; an abstention or unreadable value is left out.
fn reviewed(questions: &[&Question], verdict: &Value) -> Vec<Answer> {
    questions
        .iter()
        .filter_map(|question| {
            let value = verdict.get(&question.id)?.as_str()?;
            let answer = match (&question.kind, value) {
                (QuestionKind::Noul, "yes") => AnswerValue::Noul(1.0),
                (QuestionKind::Noul, "no") => AnswerValue::Noul(0.0),
                (QuestionKind::Choice { options }, value)
                    if options.iter().any(|option| option.value == value) =>
                {
                    AnswerValue::Choice(value.into())
                }
                (QuestionKind::Score { levels }, value) => {
                    let level: usize = value.parse().ok()?;
                    (level < levels.len()).then_some(())?;
                    AnswerValue::Score(level as f32)
                }
                _ => return None,
            };

            Some(Answer {
                id: question.id.clone(),
                value: answer,
                confidence: Some(1.0),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use chauffeur_core::{ChoiceOption, Facts, Reasoned, ReasoningFailure};
    use serde_json::json;

    use super::*;

    struct Fake {
        name: &'static str,
        replies: VecDeque<Result<Vec<Answer>, SystemOneError>>,
        asked: Arc<Mutex<Vec<Vec<String>>>>,
    }

    impl SystemOne for Fake {
        fn name(&self) -> &str {
            self.name
        }

        fn ask(&mut self, _: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
            self.asked
                .lock()
                .unwrap()
                .push(questions.iter().map(|q| q.id.clone()).collect());
            self.replies.pop_front().expect("unexpected call")
        }
    }

    struct Two(VecDeque<Reasoned>, Arc<Mutex<Vec<Reasoning>>>);

    impl SystemTwo for Two {
        fn name(&self) -> &str {
            "two"
        }

        fn reason(&mut self, request: &Reasoning) -> Reasoned {
            self.1.lock().unwrap().push(request.clone());
            self.0.pop_front().expect("unexpected review")
        }
    }

    type Log = Arc<Mutex<Vec<Vec<String>>>>;

    fn fake(
        name: &'static str,
        replies: Vec<Result<Vec<Answer>, SystemOneError>>,
    ) -> (Box<dyn SystemOne>, Log) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        (
            Box::new(Fake {
                name,
                replies: replies.into(),
                asked: asked.clone(),
            }),
            asked,
        )
    }

    fn noul(id: &str) -> Question {
        Question {
            id: id.into(),
            instructions: format!("{id}?"),
            kind: QuestionKind::Noul,
            facts: Facts::new(),
        }
    }

    fn choice(id: &str) -> Question {
        Question {
            id: id.into(),
            instructions: "Pick".into(),
            kind: QuestionKind::Choice {
                options: ["a", "b"]
                    .map(|value| ChoiceOption {
                        value: value.into(),
                        description: value.to_uppercase(),
                    })
                    .to_vec(),
            },
            facts: Facts::new(),
        }
    }

    fn answer(id: &str, p: f32) -> Answer {
        Answer {
            id: id.into(),
            value: AnswerValue::Noul(p),
            confidence: None,
        }
    }

    #[test]
    fn only_unsure_answers_escalate_and_the_second_judges_answers_stand() {
        let (first, _) = fake(
            "jev",
            vec![Ok(vec![answer("sure", 0.95), answer("unsure", 0.6)])],
        );
        let (second, asked) = fake("glide", vec![Ok(vec![answer("unsure", 0.1)])]);
        let mut judge = Escalate::new(first, second, 0.6, None);

        let answers = judge.ask("state", &[noul("sure"), noul("unsure")]).unwrap();

        assert_eq!(*asked.lock().unwrap(), vec![vec!["unsure".to_string()]]);
        assert_eq!(answers[0].value, AnswerValue::Noul(0.95));
        assert_eq!(answers[1].value, AnswerValue::Noul(0.1));
        assert_eq!(judge.name(), "jev+glide");
        assert_eq!(judge.take_replaced(), vec![answer("unsure", 0.6)]);
        assert!(judge.take_replaced().is_empty());
    }

    #[test]
    fn an_urgent_ask_takes_the_first_judges_answers_without_escalating() {
        let (first, _) = fake("jev", vec![Ok(vec![answer("q", 0.5)])]);
        let (second, asked) = fake("glide", vec![]);
        let mut judge = Escalate::new(first, second, 0.6, None);
        let deadline = Instant::now() + Duration::from_secs(2);

        let answers = judge.ask_first("s", &[noul("q")], deadline).unwrap();

        assert_eq!(answers[0].value, AnswerValue::Noul(0.5));
        assert!(asked.lock().unwrap().is_empty());
        assert_eq!(judge.name(), "jev");
        assert!(judge.take_replaced().is_empty());
    }

    #[test]
    fn confident_answers_never_reach_the_second_judge() {
        let (first, _) = fake("jev", vec![Ok(vec![answer("q", 0.05)])]);
        let (second, asked) = fake("glide", vec![]);
        let mut judge = Escalate::new(first, second, 0.6, None);

        judge.ask("state", &[noul("q")]).unwrap();

        assert!(asked.lock().unwrap().is_empty());
        assert_eq!(judge.name(), "jev");
    }

    #[test]
    fn a_failed_or_invalid_second_judge_keeps_the_first_answers() {
        for reply in [
            Err(SystemOneError("HTTP 503".into())),
            Ok(vec![answer("q", 2.0)]),
        ] {
            let (first, _) = fake("jev", vec![Ok(vec![answer("q", 0.5)])]);
            let (second, _) = fake("glide", vec![reply]);
            let mut judge = Escalate::new(first, second, 0.6, None);

            assert_eq!(
                judge.ask("s", &[noul("q")]).unwrap()[0].value,
                AnswerValue::Noul(0.5)
            );
        }
    }

    #[test]
    fn a_first_judge_failure_is_the_judgments_failure() {
        let (first, _) = fake("jev", vec![Err(SystemOneError("HTTP 500".into()))]);
        let (second, asked) = fake("glide", vec![]);

        assert!(
            Escalate::new(first, second, 0.6, None)
                .ask("s", &[noul("q")])
                .is_err()
        );
        assert!(asked.lock().unwrap().is_empty());
    }

    #[test]
    fn answers_both_judges_doubt_go_to_review_and_abstentions_keep_the_second() {
        let (first, _) = fake(
            "jev",
            vec![Ok(vec![
                answer("a", 0.5),
                answer("b", 0.5),
                Answer {
                    id: "c".into(),
                    value: AnswerValue::Choice("a".into()),
                    confidence: Some(0.2),
                },
            ])],
        );
        let (second, _) = fake(
            "glide",
            vec![Ok(vec![
                answer("a", 0.55),
                answer("b", 0.45),
                Answer {
                    id: "c".into(),
                    value: AnswerValue::Choice("a".into()),
                    confidence: Some(0.3),
                },
            ])],
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let two = Two(
            vec![Ok(json!({ "a": "yes", "b": "unsure", "c": "b" }))].into(),
            seen.clone(),
        );
        let review = Review {
            executor: Box::new(two),
            profile: "system-one-review".into(),
        };
        let mut judge = Escalate::new(first, second, 0.6, Some(review));

        let answers = judge
            .ask("state", &[noul("a"), noul("b"), choice("c")])
            .unwrap();

        assert_eq!(answers[0].value, AnswerValue::Noul(1.0));
        assert_eq!(answers[1].value, AnswerValue::Noul(0.45));
        assert_eq!(answers[2].value, AnswerValue::Choice("b".into()));
        assert_eq!(judge.name(), "jev+glide+system-two");
        // The first judge's own answers are kept, not GLiDE's in between.
        let replaced = judge.take_replaced();
        assert_eq!(replaced.len(), 3);
        assert!(replaced.iter().all(|answer| match &answer.value {
            AnswerValue::Noul(p) => *p == 0.5,
            AnswerValue::Choice(_) => answer.confidence == Some(0.2),
            AnswerValue::Score(_) => false,
        }));
        let request = &seen.lock().unwrap()[0];
        assert_eq!(request.profile, "system-one-review");
        assert!(request.instructions.contains("a = A; b = B"));
        assert_eq!(request.evidence[0].text, "state");
    }

    #[test]
    fn a_failed_review_or_no_time_keeps_the_second_answers() {
        let (first, _) = fake(
            "jev",
            vec![Ok(vec![answer("a", 0.5)]), Ok(vec![answer("a", 0.5)])],
        );
        let (second, _) = fake("glide", vec![Ok(vec![answer("a", 0.55)])]);
        let two = Two(vec![Err(ReasoningFailure::TimedOut)].into(), Arc::default());
        let review = Review {
            executor: Box::new(two),
            profile: "p".into(),
        };
        let mut judge = Escalate::new(first, second, 0.6, Some(review));

        assert_eq!(
            judge.ask("s", &[noul("a")]).unwrap()[0].value,
            AnswerValue::Noul(0.55)
        );

        // Past its deadline, nothing beyond the first judge is asked.
        let soon = Instant::now() + Duration::from_millis(100);
        assert_eq!(
            judge.ask_by("s", &[noul("a")], soon).unwrap()[0].value,
            AnswerValue::Noul(0.5)
        );
        assert_eq!(judge.name(), "jev");
    }

    #[test]
    fn a_long_state_keeps_its_most_recent_activity_within_the_input_limit() {
        let state = format!("old{}recent é", "x".repeat(MAX_REASONING_INPUT_BYTES));
        let question = noul("q");
        let request = request("p", &state, &[&question]);

        assert!(request.input_bytes() <= MAX_REASONING_INPUT_BYTES);
        assert!(request.evidence[0].text.starts_with(OMITTED));
        assert!(request.evidence[0].text.ends_with("recent é"));
        assert_eq!(request.schema.check(), Ok(()));
    }
}
